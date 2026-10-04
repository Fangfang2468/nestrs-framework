//! 配置库的装配与读取门面。
//!
//! 构建阶段按来源顺序执行一次同步加载，合并并检查预算后，才发布不可变的根节点。
//! 读取阶段由本库访问这份快照并调用 Serde，不隐式重新加载来源或创建后台任务。
//! `Configuration` 与 `ConfigSection` 通过 Arc 共享节点；它们缓存的是原始配置树，
//! 每次类型化读取都重新执行绑定，不缓存或复用绑定结果。自定义反序列化器是否执行
//! I/O 或使用业务缓存，由调用方自己的实现决定。

use crate::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigOrigins, ConfigPath, PathSegment, de,
    model::{self, Node, Value},
};
use serde::de::DeserializeOwned;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

/// 可由其他配置客户端实现的最小类型化读取协议。
///
/// `DeserializeOwned` 要求返回值拥有自己的数据，不借用客户端内部存储。两个方法都
/// 使用静态泛型分派，因此这个 trait 不支持 `dyn ConfigService`；具体客户端不必
/// 采用本库的节点、来源或合并实现。
pub trait ConfigService: Send + Sync {
    /// 读取可选值：只有路径不存在时返回 `None`，非法路径或转换失败仍返回错误。
    ///
    /// 已存在的 null 由目标类型解释，例如 `T = Option<String>` 时返回 `Some(None)`。
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, ConfigError>;
    /// 读取必需值；默认实现复用 `get`，只将外层缺失转换为 Missing 错误。
    /// 实现相对路径视图的客户端应保留自己的完整错误路径，必要时覆盖此默认方法。
    fn get_required<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        self.get(path)?.ok_or_else(|| ConfigError::missing(path))
    }
}

/// 官方客户端的配置来源扩展协议。
///
/// 来源只负责返回独立的一层，不能访问或修改先前的合并结果；跨层优先级由 builder
/// 的添加顺序决定。它与 `ConfigService` 的职责不同，实现其他读取客户端不必实现本 trait。
pub trait ConfigSource: Send + Sync {
    /// 在构建时同步加载一次；使用本次固定的目录上下文，并返回已构造好的配置层。
    /// 自定义来源的错误也应使用安全类别和定位元数据，不透传原始配置值。
    fn load(&self, context: &LoadContext) -> Result<ConfigLayer, ConfigError>;
}

/// 一次 build 内所有来源共用的加载上下文。
/// 不提供可变合并树，也不允许来源改变其他来源的路径基准。
#[derive(Clone, Debug)]
pub struct LoadContext {
    /// 构建开始时解析出的基础目录；后续来源不再各自捕获工作目录。
    base_dir: PathBuf,
}
impl LoadContext {
    /// 取得本次加载固定的基础目录。
    pub fn base_dir(&self) -> &Path {
        &self.base_dir
    }
    /// 绝对路径原样返回，相对路径连接到基础目录。
    /// 该方法只做路径组合，不检查文件、不解析符号链接，也不改变工作目录。
    pub fn resolve_path(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_owned()
        } else {
            self.base_dir.join(path)
        }
    }
}

/// 拥有来源实例的顺序构建器；添加来源本身不触发 I/O。
///
/// 默认从空对象开始；build 消费构建器，任何一步失败都不会交付部分合并结果。
#[derive(Default)]
pub struct ConfigurationBuilder {
    /// 调用方指定的目录，允许相对路径，留到 build 时统一解析。
    base_dir: Option<PathBuf>,
    /// Box 擦除不同来源的具体类型；这里只擦除对象安全的加载协议。
    sources: Vec<Box<dyn ConfigSource>>,
}
impl ConfigurationBuilder {
    /// 指定文件来源的共同基准目录；相对目录在 build 开始时基于工作目录解析。
    pub fn base_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(path.into());
        self
    }
    /// 按优先级顺序保存来源，越晚添加的层优先级越高。
    /// 来源必须拥有被保存的数据；这里不调用 load，也不读取部署环境。
    pub fn add_source<S: ConfigSource + 'static>(mut self, source: S) -> Self {
        self.sources.push(Box::new(source));
        self
    }
    /// 同步完成加载、合并和预算检查，成功后发布不可变快照。
    /// 文件来源的 I/O 会阻塞当前调用线程；本库不自动创建线程或异步运行时。
    /// 任何来源、合并或预算错误都会提前返回，尚未调用的来源不会继续执行。
    pub fn build(self) -> Result<Configuration, ConfigError> {
        // 一次性捕获 cwd，保证同一次 build 中相对 base_dir 和各文件使用同一基准。
        let cwd = std::env::current_dir().map_err(|_| ConfigError::new(ConfigErrorKind::Io))?;
        let base_dir = match self.base_dir {
            Some(path) if path.is_absolute() => path,
            Some(path) => cwd.join(path),
            None => cwd,
        };
        let context = LoadContext { base_dir };
        let mut root = ConfigLayer::empty().root;
        for source in self.sources {
            let layer = source.load(&context)?;
            // 同时限制每个输入层和累积结果：两个各自合法的层合并后仍可能超限。
            model::validate(&layer.root)?;
            model::merge(&mut root, layer.root, &ConfigPath::root())?;
            model::validate(&root)?;
        }
        // 仅在全部来源成功后建立共享快照，失败路径不会暴露半成品。
        Ok(Configuration {
            root: Arc::new(root),
        })
    }
}
impl fmt::Debug for ConfigurationBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 自定义来源不要求 Debug，也绝不调用它们可能泄漏配置内容的格式化实现。
        f.debug_struct("ConfigurationBuilder")
            .field("sources", &self.sources.len())
            .finish_non_exhaustive()
    }
}

/// 已完成加载与合并的不可变配置快照。
///
/// Clone 只共享根节点，不重新加载来源。读取时重新绑定目标类型，不缓存结果；本类型自身的 Debug
/// 隐藏配置值，但不会改变调用方取得的普通结构体或字符串的 Debug 行为。
#[derive(Clone)]
pub struct Configuration {
    /// 共享原始节点树，所有持有者都只能通过不可变引用访问它。
    root: Arc<Node>,
}
impl Configuration {
    /// 创建没有来源的构建器；未添加来源时也可构建一个空根对象。
    pub fn builder() -> ConfigurationBuilder {
        ConfigurationBuilder::default()
    }
    /// 按快捷点路径读取可选值；空字符串选择整个根对象。
    /// 路径缺失与显式 null 分开处理，类型错误不会被转换为 None。
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, ConfigError> {
        self.get_path(&ConfigPath::parse(path)?)
    }
    /// 按快捷点路径读取必需值；缺失返回 Missing，绑定错误原样保留安全分类。
    pub fn get_required<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        self.get_required_path(&ConfigPath::parse(path)?)
    }
    /// 按结构化路径读取，可明确选择带点、括号的字面键和数组元素。
    /// 节点存在才执行反序列化，返回值不借用配置树。
    pub fn get_path<T: DeserializeOwned>(
        &self,
        path: &ConfigPath,
    ) -> Result<Option<T>, ConfigError> {
        // transpose 保留“缺失”和“节点存在但绑定失败”的区别。
        lookup(&self.root, path)?
            .map(|node| de::decode(node, path))
            .transpose()
    }
    /// 显式路径版必需读取；诊断保留结构化段身份，避免字面键被误拆分。
    /// 缺失错误保留完整请求路径，形态或绑定错误定位到实际失败位置。
    pub fn get_required_path<T: DeserializeOwned>(
        &self,
        path: &ConfigPath,
    ) -> Result<T, ConfigError> {
        self.get_path(path)?
            .ok_or_else(|| ConfigError::new(ConfigErrorKind::Missing).at_path(path.clone()))
    }
    /// 立即取得必需子树；非法路径、缺失或中途形态不匹配在此时报错。
    pub fn section(&self, path: &str) -> Result<ConfigSection, ConfigError> {
        self.section_path(&ConfigPath::parse(path)?)
    }
    /// 创建共享当前快照的拥有所有权视图，不复制子树或重新加载来源。
    pub fn section_path(&self, path: &ConfigPath) -> Result<ConfigSection, ConfigError> {
        required_node(&self.root, path)?;
        Ok(ConfigSection {
            configuration: self.clone(),
            path: path.clone(),
        })
    }
    /// 解释快捷路径的最终来源，不返回原始配置值；目标必须存在。
    pub fn explain(&self, path: &str) -> Result<ConfigExplanation, ConfigError> {
        self.explain_path(&ConfigPath::parse(path)?)
    }
    /// 解释显式路径，并明确标记覆盖历史不可用；不从来源摘要推测历史。
    pub fn explain_path(&self, path: &ConfigPath) -> Result<ConfigExplanation, ConfigError> {
        let node = required_node(&self.root, path)?;
        Ok(ConfigExplanation {
            path: path.clone(),
            origins: node.origins.clone(),
            history: HistoryStatus::Unavailable,
        })
    }
}
impl ConfigService for Configuration {
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, ConfigError> {
        self.get(path)
    }
}
impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Configuration { values: <omitted> }")
    }
}

/// 拥有同一快照的子树视图，不借用创建它的临时 Configuration 变量。
///
/// 相对路径以结构化段拼接，含点的字面键不会被重新解析。视图可指向对象、数组、
/// 标量或 null；能否进一步读取字段或绑定为目标类型，由实际节点形态决定。
#[derive(Clone)]
pub struct ConfigSection {
    /// 持有根快照的 Arc，保证父门面释放后视图仍可使用。
    configuration: Configuration,
    /// 相对于根的绝对地址，用于定位节点及补齐绑定错误中的父路径。
    path: ConfigPath,
}
impl ConfigSection {
    /// 从当前子树按相对点路径读取可选值；空字符串读取当前节点。
    pub fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, ConfigError> {
        self.get_path(&ConfigPath::parse(path)?)
    }
    /// 从当前子树读取必需值；有效相对路径会组合成绝对诊断位置。
    pub fn get_required<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        self.get_required_path(&ConfigPath::parse(path)?)
    }
    /// 拼接结构化相对路径后读取，不把字面键转换为点路径字符串。
    pub fn get_path<T: DeserializeOwned>(
        &self,
        path: &ConfigPath,
    ) -> Result<Option<T>, ConfigError> {
        self.configuration.get_path(&self.path.joined(path))
    }
    /// 显式相对路径版必需读取，复用根快照的缺失、形态与转换规则。
    pub fn get_required_path<T: DeserializeOwned>(
        &self,
        path: &ConfigPath,
    ) -> Result<T, ConfigError> {
        self.configuration
            .get_required_path(&self.path.joined(path))
    }
    /// 将当前节点完整反序列化为新的 T，不运行独立业务校验。
    /// 视图创建时已经验证节点存在；不可变快照保证该节点不会在之后消失。
    pub fn bind<T: DeserializeOwned>(&self) -> Result<T, ConfigError> {
        self.configuration.get_required_path(&self.path)
    }
    /// 在当前视图下取得另一个必需子树，继续共享相同的原始快照。
    pub fn section(&self, path: &str) -> Result<Self, ConfigError> {
        self.section_path(&ConfigPath::parse(path)?)
    }
    /// 使用显式相对地址创建子视图，保留所有字面键与数组索引的身份。
    pub fn section_path(&self, path: &ConfigPath) -> Result<Self, ConfigError> {
        self.configuration.section_path(&self.path.joined(path))
    }
    /// 解释当前视图下的相对路径，输出仍使用相对于根的地址。
    pub fn explain(&self, path: &str) -> Result<ConfigExplanation, ConfigError> {
        self.explain_path(&ConfigPath::parse(path)?)
    }
    /// 显式相对地址版来源解释，不返回节点值或重新访问来源。
    pub fn explain_path(&self, path: &ConfigPath) -> Result<ConfigExplanation, ConfigError> {
        self.configuration.explain_path(&self.path.joined(path))
    }
}
impl ConfigService for ConfigSection {
    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<Option<T>, ConfigError> {
        self.get(path)
    }
    // 必须覆盖默认实现：默认方法只知道相对输入，无法为 Missing 补齐视图父路径。
    fn get_required<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigError> {
        self.get_required(path)
    }
}
impl fmt::Debug for ConfigSection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConfigSection { values: <omitted> }")
    }
}

/// 覆盖历史的可用性，与“当前节点来自哪里”分别表达。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HistoryStatus {
    /// 第一版不保留被覆盖的值或来源历史；不能据此推断没有发生过覆盖。
    Unavailable,
}

/// 只包含定位与来源的诊断视图，完全不包含配置原值。
/// 合并对象可能有多个有效来源；这不是按加载顺序保留的完整覆盖日志。
#[derive(Clone, Debug)]
pub struct ConfigExplanation {
    /// 被解释节点相对于快照根的结构化地址。
    path: ConfigPath,
    /// 实际保留节点的来源摘要，不伪造未知的文件位置。
    origins: ConfigOrigins,
    /// 明确标记历史数据是否可用，避免以空列表冒充没有覆盖。
    history: HistoryStatus,
}
impl ConfigExplanation {
    /// 取得被解释节点相对于根的结构化地址。
    pub fn path(&self) -> &ConfigPath {
        &self.path
    }
    /// 取得当前有效来源摘要；Merged 表示多个来源共同贡献结果。
    pub fn origins(&self) -> &ConfigOrigins {
        &self.origins
    }
    /// 取得覆盖历史的可用性；当前实现总是 Unavailable。
    pub fn history(&self) -> HistoryStatus {
        self.history
    }
}

/// 查询必需原始节点的内部入口：只提升缺失错误，不吞掉路径形态错误。
fn required_node<'a>(root: &'a Node, path: &ConfigPath) -> Result<&'a Node, ConfigError> {
    lookup(root, path)?
        .ok_or_else(|| ConfigError::new(ConfigErrorKind::Missing).at_path(path.clone()))
}
/// 迭代访问路径段；对象只接受 Key，数组只接受 Index。
/// 缺少对象键或数组越界是缺失，在标量/null 上继续前进则是形态错误。
fn lookup<'a>(root: &'a Node, path: &ConfigPath) -> Result<Option<&'a Node>, ConfigError> {
    let mut node = root;
    let mut traversed = ConfigPath::root();
    for segment in path.segments() {
        let next = match (&node.value, segment) {
            (Value::Object(object), PathSegment::Key(key)) => object.get(key),
            (Value::Array(array), PathSegment::Index(index)) => array.get(*index),
            _ => {
                // 定位到无法继续前进的实际容器，不能把完整请求伪装成一个缺失字段。
                let mut error = ConfigError::new(ConfigErrorKind::TypeMismatch).at_path(traversed);
                if let Some(origin) = node.origins.single() {
                    error = error.with_origin(origin.clone());
                }
                return Err(error);
            }
        };
        let Some(next) = next else {
            return Ok(None);
        };
        node = next;
        traversed.0.push(segment.clone());
    }
    Ok(Some(node))
}
