//! 官方客户端的输入值、配置层、合并规则及有效来源汇总。
//!
//! 值树保持整数精度和文本来源的类型语义，默认格式化不展开载荷。输入层与合并后的树
//! 都受深度和节点预算限制；超限值的释放使用工作栈，避免错误处理本身导致栈溢出。

use crate::{ConfigError, ConfigErrorKind, ConfigPath, MAX_DEPTH, MAX_NODES, Origin, PathSegment};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// 当前节点及其有效子树的来源，不记录已被覆盖的数据历史。
///
/// 对象递归合并后可能同时包含多个来源。只有能够确定唯一来源时，调用方才可以把它作为
/// 单一来源展示；空的可选来源不会凭空产生来源身份。
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum ConfigOrigins {
    /// 没有足够的信息确定来源，例如未添加任何来源的空配置。
    #[default]
    Unknown,
    /// 整个节点只对应一个已知来源；精确位置存在时一并保留。
    Single(Origin),
    /// 有效子节点来自多个来源；按来源身份去重，不伪造共同的行列位置。
    Merged(Vec<Origin>),
}
impl ConfigOrigins {
    /// 仅在唯一已知来源时返回引用；未知或合并来源返回 `None`。
    pub fn single(&self) -> Option<&Origin> {
        match self {
            Self::Single(origin) => Some(origin),
            _ => None,
        }
    }
}

/// 官方客户端的内部值域；保留文本来源与已定型字符串的区别，避免绑定时猜测类型。
#[derive(Clone)]
pub(crate) enum Value {
    /// 显式空值；与路径不存在保持不同语义。
    Null,
    /// 来源已经确认的布尔值。
    Bool(bool),
    /// 有符号整数，不经浮点中转。
    I64(i64),
    /// 无符号整数，覆盖 i64 无法容纳的正整数范围。
    U64(u64),
    /// 构造时已检查有限性的浮点值。
    F64(f64),
    /// 已定型字符串，不自动转为整数或布尔值。
    String(String),
    /// 环境等来源提供的原始文本，按 Serde 请求的标量类型转换。
    Text(String),
    /// 按索引定位的完整数组；跨层合并时整体替换。
    Array(Vec<Node>),
    /// 按键稳定排序的对象，为合并和 Map 条目诊断提供确定的遍历顺序。
    Object(BTreeMap<String, Node>),
}
/// 一个内部节点，将配置载荷和用于诊断的来源分开保存。
#[derive(Clone)]
pub(crate) struct Node {
    /// 配置载荷；默认 Debug 永不展开此字段。
    pub value: Value,
    /// 当前有效数据的来源信息，不持有原始错误或覆盖历史。
    pub origins: ConfigOrigins,
}
impl Node {
    fn new(value: Value) -> Self {
        Self {
            value,
            origins: ConfigOrigins::Unknown,
        }
    }
}
// 用户可以在交给构造器检查前创建任意深的输入值，超限失败时也必须安全释放。
// 先取走子节点再通过工作栈逐个释放，防止 Rust 默认的递归析构在错误路径上耗尽栈。
// 已弹出的节点被替换成 Null，其自身再次执行 Drop 时不会继续递归进入原子树。
impl Drop for Node {
    fn drop(&mut self) {
        fn children(value: Value, stack: &mut Vec<Node>) {
            match value {
                Value::Array(nodes) => stack.extend(nodes),
                Value::Object(nodes) => stack.extend(nodes.into_values()),
                _ => {}
            }
        }
        let mut stack = Vec::new();
        children(std::mem::replace(&mut self.value, Value::Null), &mut stack);
        while let Some(mut node) = stack.pop() {
            children(std::mem::replace(&mut node.value, Value::Null), &mut stack);
        }
    }
}
impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Node { value: <omitted> }")
    }
}

/// 为官方客户端的自定义来源构造输入值。
///
/// 该类型只属于 [`crate::ConfigSource`] 扩展面，不是其他 [`crate::ConfigService`]
/// 实现必须采用的统一表示。值在构造层时接受完整预算检查；默认 Debug 隐藏载荷。
#[derive(Clone)]
pub struct LayerValue(pub(crate) Node);
impl LayerValue {
    /// 构造显式 null；读取到 `Option<T>` 时对应 `None`，不会当作缺失路径。
    pub fn null() -> Self {
        Self(Node::new(Value::Null))
    }
    /// 构造已定型的布尔值。
    pub fn boolean(value: bool) -> Self {
        Self(Node::new(Value::Bool(value)))
    }
    /// 构造有符号整数，保留完整 i64 精度。
    pub fn integer(value: i64) -> Self {
        Self(Node::new(Value::I64(value)))
    }
    /// 构造无符号整数，保留完整 u64 精度。
    pub fn unsigned(value: u64) -> Self {
        Self(Node::new(Value::U64(value)))
    }
    /// 构造有限浮点值；NaN 和正负无穷返回 [`ConfigErrorKind::UnsupportedValue`]。
    pub fn float(value: f64) -> Result<Self, ConfigError> {
        if !value.is_finite() {
            return Err(ConfigError::new(ConfigErrorKind::UnsupportedValue));
        }
        Ok(Self(Node::new(Value::F64(value))))
    }
    /// 构造已定型字符串；即使内容为 `"42"`，绑定时也不会自动转换为整数。
    pub fn string(value: impl Into<String>) -> Self {
        Self(Node::new(Value::String(value.into())))
    }
    /// 构造待按目标类型转换的文本，例如环境变量值。
    ///
    /// 字符串目标保留原文；整数和布尔等直接标量请求由反序列化器检查语法与范围。
    /// `deserialize_any` 仍保持字符串语义，不从文本内容猜测数组、对象或 null。
    pub fn text(value: impl Into<String>) -> Self {
        Self(Node::new(Value::Text(value.into())))
    }
    /// 构造完整数组，保持元素顺序；不会在这里展开元素或按配置路径合并。
    pub fn array(values: Vec<Self>) -> Self {
        Self(Node::new(Value::Array(
            values.into_iter().map(|v| v.0).collect(),
        )))
    }
    /// 构造对象并按键排序；任意重复字面键返回 [`ConfigErrorKind::DuplicateKey`]。
    ///
    /// 键保持字面身份，不解析点号或数字。接受键值序列而不是已去重的 Map，才能在此处
    /// 发现重复输入，避免静默覆盖同层配置。
    pub fn object(fields: Vec<(String, Self)>) -> Result<Self, ConfigError> {
        let mut object = BTreeMap::new();
        for (key, value) in fields {
            if object.insert(key, value.0).is_some() {
                return Err(ConfigError::new(ConfigErrorKind::DuplicateKey));
            }
        }
        Ok(Self(Node::new(Value::Object(object))))
    }
    /// 为此节点附加明确来源，优先于构造层时的默认来源继承。
    ///
    /// 没有独立来源的子节点可继承该来源身份，但不会继承未经证明的精确行列。
    /// 容器最终会依据有效子节点重新汇总来源。
    pub fn with_origin(mut self, origin: Origin) -> Self {
        self.0.origins = ConfigOrigins::Single(origin);
        self
    }
}
impl fmt::Debug for LayerValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LayerValue { value: <omitted> }")
    }
}

/// 一个来源产生的配置层，根节点始终为对象。
///
/// 合法构造入口检查值域、深度和节点预算；构建配置时再按来源顺序与其他层合并。
/// 克隆复制这一输入层，配置快照的共享读取则由 [`crate::Configuration`] 负责。
#[derive(Clone)]
pub struct ConfigLayer {
    pub(crate) root: Node,
}
impl ConfigLayer {
    /// 创建没有任何配置值或来源信息的空层，适用于缺失的可选文件。
    ///
    /// 若要表达“确实加载了一个已知来源的空对象”，应使用带来源的 [`Self::builder`]。
    pub fn empty() -> Self {
        Self {
            root: Node::new(Value::Object(BTreeMap::new())),
        }
    }
    /// 创建以给定来源作为默认定位信息的层构造器。
    pub fn builder(origin: Origin) -> ConfigLayerBuilder {
        ConfigLayerBuilder {
            layer: Self::empty(),
            origin,
            inserted: BTreeSet::new(),
        }
    }
    /// 将完整对象值转换为一层，并补全来源与检查预算。
    ///
    /// 非对象根返回 [`ConfigErrorKind::TypeMismatch`]；节点自己的来源优先于 `origin`。
    pub fn from_value(value: LayerValue, origin: Origin) -> Result<Self, ConfigError> {
        let mut layer = Self { root: value.0 };
        if !matches!(layer.root.value, Value::Object(_)) {
            return Err(ConfigError::new(ConfigErrorKind::TypeMismatch).with_origin(origin));
        }
        initialize_origins(&mut layer.root, Some(origin))?;
        Ok(layer)
    }
}
impl fmt::Debug for ConfigLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConfigLayer { values: <omitted> }")
    }
}

/// 逐个插入对象路径的受检查层构造器。
///
/// 隐式父对象允许多个兄弟字段共存；显式写入同一路径及其祖先或后代则会拒绝。
/// 数组应作为完整 [`LayerValue::array`] 插入，不通过索引路径逐项构建。
pub struct ConfigLayerBuilder {
    /// 正在组装的层；只有 build 成功后才会交给配置构建器。
    layer: ConfigLayer,
    /// 没有显式来源的节点继承此来源身份。
    origin: Origin,
    /// 只记录显式插入路径，以区分合法的隐式父对象与同层父子路径冲突。
    inserted: BTreeSet<ConfigPath>,
}
impl ConfigLayerBuilder {
    /// 向对象路径插入一次值；重复路径或已有显式路径的祖先、后代均报错。
    ///
    /// 只接受对象键段。根路径只接受对象值；深度及当前子树预算在插入时检查，完整层的
    /// 节点总数和“路径加子树”的组合深度在 [`Self::build`] 时最终检查。
    pub fn insert(&mut self, path: ConfigPath, value: LayerValue) -> Result<(), ConfigError> {
        if path.segments().len() > MAX_DEPTH {
            return Err(ConfigError::new(ConfigErrorKind::Limit));
        }
        if path
            .segments()
            .iter()
            .any(|p| matches!(p, PathSegment::Index(_)))
        {
            return Err(ConfigError::new(ConfigErrorKind::InvalidPath));
        }
        // 有序路径中，第一个不小于当前路径的项即可证明是否存在后代；祖先逐级精确查找。
        // 不扫描全部已有路径，避免大量环境键插入时退化成平方复杂度。
        let descendant = self
            .inserted
            .range(path.clone()..)
            .next()
            .is_some_and(|p| p.0.starts_with(&path.0));
        let ancestor = (0..=path.0.len())
            .any(|len| self.inserted.contains(&ConfigPath(path.0[..len].to_vec())));
        if descendant || ancestor {
            return Err(ConfigError::new(ConfigErrorKind::DuplicateKey).at_path(path));
        }
        validate(&value.0)?;
        if path.0.is_empty() {
            if !matches!(value.0.value, Value::Object(_)) {
                return Err(ConfigError::new(ConfigErrorKind::TypeMismatch));
            }
            self.layer.root = value.0;
        } else {
            let mut parent = &mut self.layer.root;
            for part in &path.0[..path.0.len() - 1] {
                let PathSegment::Key(key) = part else {
                    unreachable!()
                };
                let Value::Object(object) = &mut parent.value else {
                    return Err(ConfigError::new(ConfigErrorKind::TypeMismatch));
                };
                parent = object
                    .entry(key.clone())
                    .or_insert_with(|| Node::new(Value::Object(BTreeMap::new())));
            }
            let PathSegment::Key(key) = path.0.last().unwrap() else {
                unreachable!()
            };
            let Value::Object(object) = &mut parent.value else {
                return Err(ConfigError::new(ConfigErrorKind::TypeMismatch));
            };
            object.insert(key.clone(), value.0);
        }
        self.inserted.insert(path);
        Ok(())
    }
    /// 完成整层预算检查、默认来源继承及有效来源汇总，成功后按值交付配置层。
    pub fn build(mut self) -> Result<ConfigLayer, ConfigError> {
        initialize_origins(&mut self.layer.root, Some(self.origin))?;
        Ok(self.layer)
    }
}
impl fmt::Debug for ConfigLayerBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConfigLayerBuilder { values: <omitted> }")
    }
}

// 用显式工作栈先完成预算检查与自顶向下的来源继承，再进入有深度上限的来源汇总。
// 根深度为零，根和全部容器节点均计入数量；因此路径深度与输入子树必须一起检查。
fn initialize_origins(root: &mut Node, inherited: Option<Origin>) -> Result<(), ConfigError> {
    let mut stack = vec![(&mut *root, 0usize, inherited)];
    let mut count = 0usize;
    while let Some((node, depth, inherited)) = stack.pop() {
        count += 1;
        if depth > MAX_DEPTH || count > MAX_NODES {
            return Err(ConfigError::new(ConfigErrorKind::Limit));
        }
        if matches!(node.origins, ConfigOrigins::Unknown)
            && let Some(origin) = inherited
        {
            node.origins = ConfigOrigins::Single(origin);
        }
        let child_origin = node.origins.single().map(Origin::without_position);
        match &mut node.value {
            Value::Object(values) => stack.extend(
                values
                    .values_mut()
                    .map(|node| (node, depth + 1, child_origin.clone())),
            ),
            Value::Array(values) => stack.extend(
                values
                    .iter_mut()
                    .map(|node| (node, depth + 1, child_origin.clone())),
            ),
            _ => {}
        }
    }
    refresh_origins(root);
    Ok(())
}

// 只描述合并后仍有效的子树，不积累已经被覆盖的数据来源。调用前树已通过深度预算检查，
// 因而此处递归最多走 MAX_DEPTH 层。汇总多个子节点时去除行列，避免把某个叶子的精确位置
// 冒充整个容器的位置；单一来源且节点自身原有位置可信时才保留该位置。
fn refresh_origins(node: &mut Node) {
    let children: Vec<&mut Node> = match &mut node.value {
        Value::Object(values) => values.values_mut().collect(),
        Value::Array(values) => values.iter_mut().collect(),
        _ => return,
    };
    if children.is_empty() {
        return;
    }
    let mut known = BTreeSet::new();
    let mut unknown = false;
    for child in children {
        refresh_origins(child);
        match &child.origins {
            ConfigOrigins::Unknown => unknown = true,
            ConfigOrigins::Single(origin) => {
                known.insert(origin.without_position());
            }
            ConfigOrigins::Merged(origins) => {
                known.extend(origins.iter().map(Origin::without_position))
            }
        }
    }
    // 当前合法层经继承后通常不会混合已知与未知子来源；仍保留未知分支，以免以后新增来源
    // 能力时把“不完全知道”错误地展示为完整的来源列表。
    node.origins = if unknown {
        ConfigOrigins::Unknown
    } else {
        match known.len() {
            0 => ConfigOrigins::Unknown,
            1 => {
                let origin = known.pop_first().unwrap();
                let original = node
                    .origins
                    .single()
                    .filter(|o| o.without_position() == origin);
                ConfigOrigins::Single(original.cloned().unwrap_or(origin))
            }
            _ => ConfigOrigins::Merged(known.into_iter().collect()),
        }
    };
}

/// 迭代检查整棵输入树的深度和节点数量，不对配置值进行业务规则校验。
///
/// 自定义来源返回的层和跨层合并后的树都需要检查：每层单独合法不代表合并结果仍在预算内。
pub(crate) fn validate(root: &Node) -> Result<(), ConfigError> {
    let mut stack = vec![(root, 0usize)];
    let mut count = 0usize;
    while let Some((node, depth)) = stack.pop() {
        count += 1;
        if depth > MAX_DEPTH || count > MAX_NODES {
            return Err(ConfigError::new(ConfigErrorKind::Limit));
        }
        match &node.value {
            Value::Array(values) => stack.extend(values.iter().map(|n| (n, depth + 1))),
            Value::Object(values) => stack.extend(values.values().map(|n| (n, depth + 1))),
            _ => {}
        }
    }
    Ok(())
}

/// 将高优先级节点并入低优先级节点，保持完整对象路径以便报告形态冲突。
///
/// 对象按键递归合并，数组整体替换，非 null 标量由高层替换；null 可以与任意形态互相替换。
/// 其他容器形态冲突返回错误，由调用方丢弃整个未交付的构建结果，不发布部分配置。
/// 输入层均已检查深度，因此这里的递归深度受 MAX_DEPTH 限制。
pub(crate) fn merge(low: &mut Node, mut high: Node, path: &ConfigPath) -> Result<(), ConfigError> {
    if matches!(low.value, Value::Null) || matches!(high.value, Value::Null) {
        *low = high;
        return Ok(());
    }
    match (&mut low.value, &mut high.value) {
        (Value::Object(low_map), Value::Object(high_map)) => {
            for (key, value) in std::mem::take(high_map) {
                if let Some(existing) = low_map.get_mut(&key) {
                    merge(existing, value, &path.clone().key(key))?;
                } else {
                    low_map.insert(key, value);
                }
            }
            // 显式空对象仍有已知来源；可选来源缺失形成的 Unknown 空层不能抹掉它。
            // 非空对象则按合并后真正剩余的子节点重新汇总，避免保留已失效的旧来源。
            if low_map.is_empty() {
                if !matches!(high.origins, ConfigOrigins::Unknown) {
                    low.origins = high.origins.clone();
                }
            } else {
                low.origins = ConfigOrigins::Unknown;
                refresh_origins(low);
            }
        }
        (Value::Array(_), Value::Array(_)) => {
            *low = high;
        }
        (Value::Object(_) | Value::Array(_), _) | (_, Value::Object(_) | Value::Array(_)) => {
            let mut error = ConfigError::new(ConfigErrorKind::MergeConflict).at_path(path.clone());
            if let Some(origin) = high.origins.single() {
                error = error.with_origin(origin.clone());
            }
            return Err(error);
        }
        _ => {
            *low = high;
        }
    }
    Ok(())
}
