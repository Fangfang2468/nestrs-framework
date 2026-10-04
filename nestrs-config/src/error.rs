//! 安全错误类别与来源元数据。
//!
//! 配置错误可能涉及凭据，本模块以固定类别取代任意底层错误文本。定位元数据与配置值
//! 分开保存，并在 Map 条目错误上主动移除可能编码动态键的来源信息。

use crate::{ConfigLocation, ConfigPath};
use std::{fmt, path::PathBuf};

/// 配置来源的类别；后续可增加新的来源类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum OriginKind {
    /// 文件来源，名称保存调用方提供或加载器解析后的路径。
    File,
    /// 环境变量来源，名称保存变量名。
    Environment,
    /// 内存层或第三方来源提供的逻辑名称。
    Named,
}

/// 可选的来源定位信息，不持有配置值或底层错误。
///
/// 文件路径、环境变量名和自定义名称会出现在诊断中。调用方必须确保这些定位元数据本身
/// 不包含凭据；本类型不会根据名称内容推断敏感性。未知行列保持缺省，不伪造精确位置。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Origin {
    /// 来源类别，与名称共同区分来源身份。
    kind: OriginKind,
    /// 可用于诊断的来源名称，不是原始配置内容。
    name: String,
    /// 从一开始的行号；无可靠位置时为 `None`。
    line: Option<usize>,
    /// 从一开始的列号；具体计数方式由提供位置的来源负责。
    column: Option<usize>,
}
impl Origin {
    /// 记录文件路径，不打开文件、不检查其存在性；非 Unicode 路径使用有损显示形式。
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::make(OriginKind::File, path.into().to_string_lossy().into_owned())
    }
    /// 记录环境变量名，不读取该变量的值。
    pub fn environment(name: impl Into<String>) -> Self {
        Self::make(OriginKind::Environment, name.into())
    }
    /// 记录内存层或第三方来源的安全逻辑名称。
    pub fn named(name: impl Into<String>) -> Self {
        Self::make(OriginKind::Named, name.into())
    }
    fn make(kind: OriginKind, name: String) -> Self {
        Self {
            kind,
            name,
            line: None,
            column: None,
        }
    }
    /// 附加从一开始的行列号；任一值为零时返回 [`ConfigErrorKind::InvalidOptions`]。
    ///
    /// 此方法只记录来源提供的位置，不从配置内容或其他字段推断行列。
    pub fn at(mut self, line: usize, column: usize) -> Result<Self, ConfigError> {
        if line == 0 || column == 0 {
            return Err(ConfigError::new(ConfigErrorKind::InvalidOptions));
        }
        self.line = Some(line);
        self.column = Some(column);
        Ok(self)
    }
    /// 返回来源类别。
    pub fn kind(&self) -> OriginKind {
        self.kind
    }
    /// 借用来源名称；该名称可用于诊断，但不包含配置值。
    pub fn name(&self) -> &str {
        &self.name
    }
    /// 返回已知的行号；`None` 表示来源没有提供精确位置。
    pub fn line(&self) -> Option<usize> {
        self.line
    }
    /// 返回已知的列号；`None` 表示来源没有提供精确位置。
    pub fn column(&self) -> Option<usize> {
        self.column
    }
    // 父子节点可以共享来源身份，但不能共享未经证明的精确行列；汇总来源时也按身份去重。
    pub(crate) fn without_position(&self) -> Self {
        Self {
            line: None,
            column: None,
            ..self.clone()
        }
    }
}

/// 可供程序匹配的安全错误类别，不携带被拒绝的值或第三方错误消息。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConfigErrorKind {
    /// 请求的对象键或数组元素不存在；显式 null 不属于缺失。
    Missing,
    /// 快捷路径语法无效，或该操作不支持给定路径段。
    InvalidPath,
    /// 读取时的容器形态或值类型不符合目标类型要求。
    TypeMismatch,
    /// 来源读取失败，或给定路径不是支持的普通文件。
    Io,
    /// 来源包含不支持的编码，例如非 UTF-8 文件内容或非 Unicode 环境输入。
    Encoding,
    /// 来源文本不符合所选格式的语法。
    Parse,
    /// 同层重复键、规范化后的名称碰撞或显式父子路径冲突。
    DuplicateKey,
    /// 来源或定位选项无效，例如空环境分隔符或零行列号。
    InvalidOptions,
    /// 不同层在同一路径提供了无法合并的容器形态。
    MergeConflict,
    /// Serde 绑定失败，包括业务反序列化器返回的错误；原始消息不会被保存。
    Binding,
    /// 值超出统一输入域，例如非有限浮点或不支持的 TOML 日期时间。
    UnsupportedValue,
    /// 文件大小、树深度、节点数量或解析器资源预算超限。
    Limit,
}
impl ConfigErrorKind {
    // 仅使用固定文案，避免在 Display 链路中格式化解析器或业务类型携带的任意输入。
    fn message(self) -> &'static str {
        match self {
            Self::Missing => "configuration value is missing",
            Self::InvalidPath => "invalid configuration path",
            Self::TypeMismatch => "configuration type does not match the requested type",
            Self::Io => "configuration source could not be read",
            Self::Encoding => "configuration source is not valid Unicode",
            Self::Parse => "configuration source has invalid syntax",
            Self::DuplicateKey => "configuration source contains a duplicate key or path",
            Self::InvalidOptions => "invalid configuration source options",
            Self::MergeConflict => "configuration layers have incompatible shapes",
            Self::Binding => "configuration deserialization failed",
            Self::UnsupportedValue => "configuration value is outside the supported value domain",
            Self::Limit => "configuration resource limit exceeded",
        }
    }
}

/// 只保留类别、结构化位置和可选来源的安全错误封装。
///
/// 不保存解析器、I/O 或业务反序列化器提供的任意消息，也不暴露底层错误链；
/// [`std::error::Error::source`] 返回 `None`。显式路径与来源名称仍由调用方负责保证安全。
/// 当位置包含 Map 条目序号时，来源信息会被省略，避免环境变量名等元数据再次泄漏动态键。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// 可用于分支处理的固定错误类别。
    kind: ConfigErrorKind,
    /// 可安全展示的结构化位置；无法确定时不伪造。
    location: Option<ConfigLocation>,
    /// 可安全展示的来源；Map 条目错误始终省略。
    origin: Option<Origin>,
}
impl ConfigError {
    /// 创建指定类别的错误，不附加未经确认的位置、来源或任意消息。
    pub fn new(kind: ConfigErrorKind) -> Self {
        Self {
            kind,
            location: None,
            origin: None,
        }
    }
    /// 为合法快捷路径创建缺失错误；路径本身无效时返回路径错误。
    pub fn missing(path: &str) -> Self {
        match ConfigPath::parse(path) {
            Ok(path) => Self::new(ConfigErrorKind::Missing).at_path(path),
            Err(error) => error,
        }
    }
    /// 将显式读取路径作为诊断位置附加到错误上。
    pub fn at_path(self, path: ConfigPath) -> Self {
        self.at_location(ConfigLocation::from(&path))
    }
    /// 设置诊断位置；若含 Map 条目段，同时清除已有来源。
    ///
    /// 清除操作与 [`Self::with_origin`] 的检查配合，使方法调用顺序不会绕过动态键保护。
    pub fn at_location(mut self, location: ConfigLocation) -> Self {
        if location.has_entries() {
            self.origin = None;
        }
        self.location = Some(location);
        self
    }
    /// 附加安全来源；当前位置含 Map 条目段时忽略来源，避免旁路泄漏动态键。
    pub fn with_origin(mut self, origin: Origin) -> Self {
        if !self
            .location
            .as_ref()
            .is_some_and(ConfigLocation::has_entries)
        {
            self.origin = Some(origin);
        }
        self
    }
    /// 返回固定错误类别。
    pub fn kind(&self) -> ConfigErrorKind {
        self.kind
    }
    /// 借用已知诊断位置；`None` 表示没有可靠的定位信息。
    pub fn location(&self) -> Option<&ConfigLocation> {
        self.location.as_ref()
    }
    /// 借用已知安全来源；未知来源及 Map 条目错误均返回 `None`。
    pub fn origin(&self) -> Option<&Origin> {
        self.origin.as_ref()
    }
}
impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.message())?;
        if let Some(location) = &self.location {
            write!(f, " at {location}")?;
        }
        if let Some(origin) = &self.origin {
            write!(f, " (source {:?}", origin.name)?;
            if let (Some(line), Some(column)) = (origin.line, origin.column) {
                write!(f, ":{line}:{column}")?;
            }
            f.write_str(")")?;
        }
        Ok(())
    }
}
impl std::error::Error for ConfigError {}
