//! 配置读取路径与安全诊断位置。
//!
//! 读取路径区分对象键和数组索引；诊断位置额外支持 Map 条目序号。两者各自保持段的身份，
//! 不通过解析错误消息恢复位置，也不把动态 Map 键复制到错误中。

use crate::{ConfigError, ConfigErrorKind};
use std::fmt;

/// 显式配置路径中的一个段，区分对象键与数组索引。
///
/// 对象键保留字面内容，不会再次按点号、括号或数字解析。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathSegment {
    /// 按完整字面值选择对象键；空字符串和包含点号的键也合法。
    Key(String),
    /// 按从零开始的位置选择数组元素。
    Index(usize),
}

/// 用于读取配置的结构化地址；零段地址表示根节点。
///
/// 简单对象路径可通过 [`Self::parse`] 构造；特殊字面键和数组索引应分别使用
/// [`Self::key`] 与 [`Self::index`]。路径的显示形式仅用于诊断，不承诺能够重新解析。
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ConfigPath(pub(crate) Vec<PathSegment>);

impl ConfigPath {
    /// 创建选择整个配置根节点的空路径。
    pub fn root() -> Self {
        Self::default()
    }

    /// 解析以点号分隔的对象路径；空字符串选择根节点。
    ///
    /// 数字段仍是对象键，例如 `items.0` 不表示数组索引。开头或末尾的点号、连续点号，
    /// 以及任意方括号均返回 [`ConfigErrorKind::InvalidPath`]；不会猜测转义或折叠空段。
    /// 含这些字符的字面键应改用 [`Self::key`]。
    pub fn parse(path: &str) -> Result<Self, ConfigError> {
        if path.is_empty() {
            return Ok(Self::root());
        }
        if path.contains(['[', ']']) || path.split('.').any(str::is_empty) {
            return Err(ConfigError::new(ConfigErrorKind::InvalidPath));
        }
        Ok(Self(
            path.split('.')
                .map(|key| PathSegment::Key(key.into()))
                .collect(),
        ))
    }

    /// 追加一个字面对象键，不解析其中的点号、括号或空字符串。
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.0.push(PathSegment::Key(key.into()));
        self
    }
    /// 追加从零开始的数组索引；实际读取时才检查容器类型及是否越界。
    pub fn index(mut self, index: usize) -> Self {
        self.0.push(PathSegment::Index(index));
        self
    }
    /// 按从根到叶子的顺序借用路径段，不分配或暴露配置值。
    pub fn segments(&self) -> &[PathSegment] {
        &self.0
    }
    // 子树读取通过拼接结构化段补齐绝对位置，避免特殊字面键在字符串拼接时被重新解析。
    pub(crate) fn joined(&self, relative: &Self) -> Self {
        Self(self.0.iter().chain(&relative.0).cloned().collect())
    }
}
impl fmt::Debug for ConfigPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
impl fmt::Display for ConfigPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&ConfigLocation::from(self), f)
    }
}

/// Map 条目内发生错误的位置，不包含动态键的原文。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MapRole {
    /// 错误发生在条目键的反序列化过程中。
    Key,
    /// 错误发生在对应值的反序列化过程中。
    Value,
}

/// 安全诊断位置中的一个段。
///
/// Map 使用稳定条目序号定位；序号不是对象键，不能将诊断位置当作配置读取路径。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationSegment {
    /// 字段或显式查询的字面对象键；调用方应确保名称本身不含秘密。
    Field(String),
    /// 从零开始的数组或顺序集合元素索引。
    Index(usize),
    /// 按稳定遍历顺序标识 Map 条目，不保存被校验的动态键。
    ///
    /// `index` 是从零开始的条目序号；`role` 指明错误属于该条目的键还是值。
    Entry { index: usize, role: MapRole },
}

/// 错误的结构化位置，可表达字段、数组元素及不泄漏动态键的 Map 条目。
///
/// [`Default`] 创建根位置。显示时用独立段和字符串转义保留身份，避免把字面点号误认为
/// 层级边界；该显示文本只服务于诊断，不是 [`ConfigPath::parse`] 的输入协议。
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ConfigLocation(Vec<LocationSegment>);
impl ConfigLocation {
    /// 追加已知字段名或显式查询键；这里不会识别或隐藏名称中的敏感信息。
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.0.push(LocationSegment::Field(field.into()));
        self
    }
    /// 追加从零开始的集合元素索引。
    pub fn index(mut self, index: usize) -> Self {
        self.0.push(LocationSegment::Index(index));
        self
    }
    /// 追加 Map 条目序号与角色，不接受动态键参数。
    pub fn entry(mut self, index: usize, role: MapRole) -> Self {
        self.0.push(LocationSegment::Entry { index, role });
        self
    }
    /// 借用完整诊断段，供调用方构造结构化报告。
    pub fn segments(&self) -> &[LocationSegment] {
        &self.0
    }
    /// 判断是否包含 Map 条目；安全错误据此省略可能编码动态键的来源元数据。
    pub fn has_entries(&self) -> bool {
        self.0
            .iter()
            .any(|s| matches!(s, LocationSegment::Entry { .. }))
    }
}
impl From<&ConfigPath> for ConfigLocation {
    fn from(path: &ConfigPath) -> Self {
        Self(
            path.0
                .iter()
                .map(|s| match s {
                    PathSegment::Key(key) => LocationSegment::Field(key.clone()),
                    PathSegment::Index(index) => LocationSegment::Index(*index),
                })
                .collect(),
        )
    }
}
impl fmt::Display for ConfigLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("$")?;
        for part in &self.0 {
            match part {
                LocationSegment::Field(key) => write!(f, "[{key:?}]")?,
                LocationSegment::Index(index) => write!(f, "[{index}]")?,
                LocationSegment::Entry { index, role } => write!(
                    f,
                    "{{entry#{index}}}.{}",
                    match role {
                        MapRole::Key => "key",
                        MapRole::Value => "value",
                    }
                )?,
            }
        }
        Ok(())
    }
}
impl fmt::Debug for ConfigLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
