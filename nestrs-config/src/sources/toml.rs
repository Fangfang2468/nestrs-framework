//! TOML 文件来源：使用解析器的公开值与 span，而不是丢失位置信息的通用值树。
//!
//! TOML 语法和重复声明由原生解析器检查；配置库负责限制共同值域、转换预算，
//! 并把解析器错误归一化为不含原文的安全类别。

use std::path::PathBuf;

use toml::de::{DeTable, DeValue};

use super::file::{FileInput, Positions};
use crate::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigPath, ConfigSource, LayerValue, LoadContext,
    MAX_DEPTH, MAX_NODES,
};

/// 保存节点来源位置的 UTF-8 TOML 文件来源。
///
/// 支持对象、数组和共同标量值域；TOML 原生日期时间及非有限浮点数被明确拒绝，
/// 需要这些表示时应使用字符串并由应用自己的反序列化类型解释。
#[derive(Clone, Debug)]
pub struct Toml {
    input: FileInput,
}

impl Toml {
    /// 保存 TOML 文件路径，在加载时按固定基础目录解析相对路径并读取。
    ///
    /// 创建来源不会立即访问文件，也不会搜索其他同名配置文件。
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            input: FileInput::new(path),
        }
    }

    /// 仅允许指定文件不存在；存在但不可读取、编码错误或语法错误仍然失败。
    pub fn optional(mut self) -> Self {
        self.input.optional = true;
        self
    }
}

impl ConfigSource for Toml {
    /// 使用原生 TOML 解析器取得带 span 的对象，再转换为独立配置层。
    fn load(&self, context: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        let (text, origin) = self.input.read(context)?;
        let Some(text) = text else {
            return Ok(ConfigLayer::empty());
        };
        let positions = Positions::new(&text, origin.clone());
        // 解析器自带递归保护，文件读取也先受字节上限约束。配置节点及深度预算在
        // 转换原生解析树时检查，不能把它描述成解析器瞬时内存分配的硬上限。
        let table = DeTable::parse(&text).map_err(|error| {
            // 当前依赖没有暴露这些结构化类别，只识别已核对的固定消息。
            // 分类后丢弃原始错误；递归耗尽归为资源限制，不能误报普通语法错误。
            let kind = if error.message() == "duplicate key" {
                ConfigErrorKind::DuplicateKey
            } else if matches!(
                error.message(),
                "recursion limit" | "cannot recurse further; max recursion depth met"
            ) {
                ConfigErrorKind::Limit
            } else {
                ConfigErrorKind::Parse
            };
            let located = error
                .span()
                .map(|span| positions.at(span.start))
                .unwrap_or_else(|| origin.clone());
            ConfigError::new(kind).with_origin(located)
        })?;
        let offset = table.span().start;
        let mut conversion = Conversion {
            positions,
            nodes: 0,
        };
        let value = conversion.convert(
            DeValue::Table(table.into_inner()),
            offset,
            0,
            ConfigPath::root(),
        )?;
        ConfigLayer::from_value(value, origin)
    }
}

/// 一次 TOML 转换的定位索引和累计节点数，各子树共享同一个预算。
struct Conversion {
    positions: Positions,
    nodes: usize,
}

impl Conversion {
    /// 保留值类型与原始 span 起点；在下降到子树前拒绝超出预算的输入。
    fn convert(
        &mut self,
        value: DeValue<'_>,
        offset: usize,
        depth: usize,
        path: ConfigPath,
    ) -> Result<LayerValue, ConfigError> {
        let origin = self.positions.at(offset);
        self.nodes += 1;
        if depth > MAX_DEPTH || self.nodes > MAX_NODES {
            return Err(ConfigError::new(ConfigErrorKind::Limit)
                .with_origin(origin)
                .at_path(path));
        }
        let unsupported = || {
            ConfigError::new(ConfigErrorKind::UnsupportedValue)
                .with_origin(origin.clone())
                .at_path(path.clone())
        };
        let value = match value {
            DeValue::String(value) => LayerValue::string(value.into_owned()),
            DeValue::Boolean(value) => LayerValue::boolean(value),
            DeValue::Integer(value) => {
                // 解析器提供已规范化的数字及进制，先尝试有符号范围，再覆盖合法的
                // 无符号高位区间；整个过程不经过浮点数，避免大整数精度损失。
                if let Ok(signed) = i64::from_str_radix(value.as_str(), value.radix()) {
                    LayerValue::integer(signed)
                } else {
                    LayerValue::unsigned(
                        u64::from_str_radix(value.as_str(), value.radix())
                            .map_err(|_| unsupported())?,
                    )
                }
            }
            DeValue::Float(value) => {
                let number = value.as_str().parse::<f64>().map_err(|_| unsupported())?;
                LayerValue::float(number).map_err(|_| unsupported())?
            }
            // 不把 TOML 专属日期时间偷偷转换为字符串，以免改变跨来源的类型语义。
            DeValue::Datetime(_) => return Err(unsupported()),
            DeValue::Array(values) => {
                let mut array = Vec::new();
                for (index, value) in values.into_iter().enumerate() {
                    let offset = value.span().start;
                    array.push(self.convert(
                        value.into_inner(),
                        offset,
                        depth + 1,
                        path.clone().index(index),
                    )?);
                }
                LayerValue::array(array)
            }
            DeValue::Table(values) => {
                let mut object = Vec::new();
                for (key, value) in values {
                    let key = key.into_inner().into_owned();
                    let offset = value.span().start;
                    // 使用键的实际字符串作为一个字面段，不把含点的引号键再次拆分。
                    let value = self.convert(
                        value.into_inner(),
                        offset,
                        depth + 1,
                        path.clone().key(key.clone()),
                    )?;
                    object.push((key, value));
                }
                LayerValue::object(object)?
            }
        };
        Ok(value.with_origin(origin))
    }
}
