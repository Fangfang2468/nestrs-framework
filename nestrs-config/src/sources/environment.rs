//! 将进程环境或显式环境输入映射为对象路径与待转换文本。
//!
//! 前缀按原始拼写筛选，只有路径段转为 ASCII 小写；不解析逗号、JSON 或数组索引。
//! 同层冲突、编码及资源预算在构建期间检查，整个流程从不修改进程环境。

use std::{collections::HashSet, ffi::OsString};

use crate::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigPath, ConfigSource, LayerValue, LoadContext,
    MAX_DEPTH, MAX_NODES, Origin,
};

/// 只读的环境变量来源，值在反序列化前始终保持文本语义。
///
/// 默认以 `__` 分隔对象路径。例如前缀为 `APP__` 时，`APP__HTTP__PORT` 映射为
/// `http.port`。前缀匹配区分大小写，路径中的单下划线保持不变。
/// 数字路径段仍是对象键；文本 `null`、空字符串或类似 JSON 的内容不会提前猜测为其他类型。
#[derive(Clone)]
pub struct Environment {
    prefix: String,
    separator: String,
    input: Option<Vec<(OsString, OsString)>>,
}

impl Environment {
    /// 创建在加载时读取进程环境的来源。
    ///
    /// 构造时不读取环境；每次 `load` 只枚举一次并在本次加载中使用这份输入。
    /// 空前缀表示选择所有条目，后续仍执行编码、键映射与冲突检查。
    pub fn with_prefix(prefix: impl Into<String>) -> Self {
        Self {
            prefix: prefix.into(),
            separator: "__".into(),
            input: None,
        }
    }

    /// 接管显式键值对，用于测试、预先捕获的环境或应用提供的文本覆盖。
    ///
    /// 本入口既不读取也不修改进程环境。输入保留为 `OsString`，因此可以真实测试
    /// 非 Unicode 键值；编码与重复声明在加载时检查，不在构造时静默丢弃。
    pub fn from_iter<I, K, V>(prefix: impl Into<String>, pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        Self {
            prefix: prefix.into(),
            separator: "__".into(),
            input: Some(
                pairs
                    .into_iter()
                    .map(|(key, value)| (key.into(), value.into()))
                    .collect(),
            ),
        }
    }

    /// 设置对象路径的分隔符，默认值为 `__`。
    ///
    /// 空分隔符在加载时返回选项错误；空路径段也会被拒绝，不会折叠为合法路径。
    pub fn separator(mut self, separator: impl Into<String>) -> Self {
        self.separator = separator.into();
        self
    }
}

impl ConfigSource for Environment {
    /// 捕获本次输入，规范化选中的键，并构造带环境变量来源信息的独立层。
    fn load(&self, _context: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        let source_origin = Origin::named("environment");
        validate_options(&self.separator, &source_origin)?;
        let mut pairs = match &self.input {
            Some(input) => input.clone(),
            None => std::env::vars_os().collect(),
        };
        // 排序使同一批输入的首个冲突或非法值不受操作系统枚举顺序影响。
        // 不按枚举先后挑选撞名变量，任何同层重复都由层构造器拒绝。
        pairs.sort_by(|left, right| left.0.cmp(&right.0));
        let mut layer = ConfigLayer::builder(source_origin.clone());
        let mut nodes = HashSet::from([ConfigPath::root()]);
        for (key, value) in pairs {
            // 有效 UTF-8 前缀在 OsStr 的自同步编码中具有相同字节表示。
            // 先筛选前缀，再要求 Unicode，避免无关变量的编码问题导致当前来源失败。
            if !key.as_encoded_bytes().starts_with(self.prefix.as_bytes()) {
                continue;
            }
            let key = key.into_string().map_err(|_| {
                ConfigError::new(ConfigErrorKind::Encoding).with_origin(source_origin.clone())
            })?;
            let origin = Origin::environment(key.clone());
            let value = value.into_string().map_err(|_| {
                ConfigError::new(ConfigErrorKind::Encoding).with_origin(origin.clone())
            })?;
            let path = mapped_path(&key, &self.prefix, &self.separator, &origin)?;
            // 在插入前累计实际路径节点，不能只按环境变量条数估计内存层规模。
            track_path(&mut nodes, &path, &origin)?;
            layer
                .insert(path, LayerValue::text(value).with_origin(origin.clone()))
                .map_err(|error| error.with_origin(origin))?;
        }
        layer.build()
    }
}

impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 仅展示读取选项和是否显式提供输入，不格式化持有的任何键值对。
        f.debug_struct("Environment")
            .field("prefix", &self.prefix)
            .field("separator", &self.separator)
            .field("explicit_input", &self.input.is_some())
            .finish_non_exhaustive()
    }
}

/// 环境来源与 dotenv 共用的选项检查，非法选项不会被可选文件缺失掩盖。
pub(super) fn validate_options(separator: &str, origin: &Origin) -> Result<(), ConfigError> {
    if separator.is_empty() {
        Err(ConfigError::new(ConfigErrorKind::InvalidOptions).with_origin(origin.clone()))
    } else {
        Ok(())
    }
}

/// 去掉已匹配前缀，将非空段转为 ASCII 小写，并逐段构造字面对象键。
///
/// 不将数字段解释成数组下标，也不把段内的点重新拆分；深度在构造路径时即受限制。
pub(super) fn mapped_path(
    key: &str,
    prefix: &str,
    separator: &str,
    origin: &Origin,
) -> Result<ConfigPath, ConfigError> {
    let suffix = key.strip_prefix(prefix).ok_or_else(|| {
        ConfigError::new(ConfigErrorKind::InvalidOptions).with_origin(origin.clone())
    })?;
    let mut path = ConfigPath::root();
    for (depth, segment) in suffix.split(separator).enumerate() {
        if segment.is_empty() {
            return Err(
                ConfigError::new(ConfigErrorKind::InvalidOptions).with_origin(origin.clone())
            );
        }
        if depth >= MAX_DEPTH {
            return Err(ConfigError::new(ConfigErrorKind::Limit).with_origin(origin.clone()));
        }
        path = path.key(segment.to_ascii_lowercase());
    }
    Ok(path)
}

/// 将路径的所有对象前缀计入本层节点预算，共用父对象只计一次。
///
/// 环境变量可能很少但路径很深，或大量条目共用同一个父对象；按唯一前缀计数既能
/// 及时限制构树规模，也不会因重复计算共享父节点而提前拒绝合法层。
pub(super) fn track_path(
    nodes: &mut HashSet<ConfigPath>,
    path: &ConfigPath,
    origin: &Origin,
) -> Result<(), ConfigError> {
    let mut prefix = ConfigPath::root();
    for segment in path.segments() {
        let crate::PathSegment::Key(key) = segment else {
            unreachable!("environment paths only contain object keys")
        };
        prefix = prefix.key(key);
        nodes.insert(prefix.clone());
        if nodes.len() > MAX_NODES {
            return Err(ConfigError::new(ConfigErrorKind::Limit).with_origin(origin.clone()));
        }
    }
    Ok(())
}
