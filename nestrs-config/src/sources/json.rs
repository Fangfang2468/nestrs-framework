//! JSON 文件来源：保留数字精度、检测每层重复键，并建立真实来源位置。
//!
//! 先以借用的 `RawValue` 验证 JSON 语法，再根据原始数字片段构造配置值，避免
//! 大整数先转成浮点数或普通 Map 提前覆盖重复键。公开错误不保留解析器原始消息。

use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserializer as _;
use serde::de::{Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::value::RawValue;

use super::file::{FileInput, Positions};
use crate::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigPath, ConfigSource, LayerValue, LoadContext,
    MAX_DEPTH, MAX_NODES, Origin,
};

/// 根节点必须为对象的 UTF-8 JSON 文件来源。
///
/// 任意层级的重复键、超出整数值域的数以及非有限浮点数都会被拒绝。
/// 文件字符串保持字符串类型，不因外观像数字就自动转为数值。
#[derive(Clone, Debug)]
pub struct Json {
    input: FileInput,
}

impl Json {
    /// 保存 JSON 文件路径，实际读取在构建器加载来源时进行。
    ///
    /// 相对路径由本次加载上下文的基础目录解析，不搜索父目录或隐式配置文件。
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            input: FileInput::new(path),
        }
    }

    /// 将文件标记为可选，仅忽略 `NotFound`。
    ///
    /// 文件存在但语法、编码、类型或读取权限错误时仍失败；缺失不贡献来源元数据。
    pub fn optional(mut self) -> Self {
        self.input.optional = true;
        self
    }
}

impl ConfigSource for Json {
    /// 读取文件、验证语法，并在节点及深度预算内转换为独立配置层。
    fn load(&self, context: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        let (text, origin) = self.input.read(context)?;
        let Some(text) = text else {
            return Ok(ConfigLayer::empty());
        };
        // 初次解析确保 null/bool 等字面量和整体语法有效；随后按片段首字符分派时，
        // 不需要自己重新实现 JSON 词法，也不会先将整数转换成 f64。
        let raw = serde_json::from_str::<&RawValue>(&text)
            .map_err(|error| parse_error(error, &origin))?;
        let mut conversion = Conversion {
            text: &text,
            positions: Positions::new(&text, origin.clone()),
            nodes: 0,
        };
        let value = conversion.convert(raw, 0, ConfigPath::root())?;
        ConfigLayer::from_value(value, origin)
    }
}

/// 只提取解析器的行列信息，丢弃可能包含原始值或文件片段的错误文本。
fn parse_error(error: serde_json::Error, origin: &Origin) -> ConfigError {
    let located = origin
        .clone()
        .at(error.line().max(1), error.column().max(1))
        .unwrap_or_else(|_| origin.clone());
    ConfigError::new(ConfigErrorKind::Parse).with_origin(located)
}

/// 同一份文件的转换状态；所有递归分支共用节点计数，避免各分支单独重置预算。
struct Conversion<'text> {
    text: &'text str,
    positions: Positions,
    nodes: usize,
}

impl<'text> Conversion<'text> {
    /// 将已验证语法的借用片段转换为一个配置节点，并记录其实际起始位置。
    fn convert(
        &mut self,
        raw: &'text RawValue,
        depth: usize,
        path: ConfigPath,
    ) -> Result<LayerValue, ConfigError> {
        let text = raw.get();
        // RawValue 直接借用同一份原始输入，片段与整份文本的地址差就是字节偏移。
        // 这里只计算位置，不延长借用，也不构造越界引用。
        let offset = (text.as_ptr() as usize).saturating_sub(self.text.as_ptr() as usize);
        let origin = self.positions.at(offset);
        self.nodes += 1;
        // 在向子节点展开前检查累计预算；根节点深度为 0，数组和对象都计入节点数。
        if depth > MAX_DEPTH || self.nodes > MAX_NODES {
            return Err(ConfigError::new(ConfigErrorKind::Limit)
                .with_origin(origin)
                .at_path(path));
        }
        let error = |kind| {
            ConfigError::new(kind)
                .with_origin(origin.clone())
                .at_path(path.clone())
        };
        let value = match text.as_bytes().first().copied() {
            Some(b'{') => {
                // Serde Visitor 必须返回解析器错误类型；单独保留安全 ConfigError，
                // 外层优先恢复具体类别，不把 Visitor 的失败重新包装成任意原始消息。
                let mut failure = None;
                let mut deserializer = serde_json::Deserializer::from_str(text);
                let result = deserializer.deserialize_map(ObjectVisitor {
                    conversion: self,
                    depth,
                    path: path.clone(),
                    origin: origin.clone(),
                    failure: &mut failure,
                });
                match result {
                    Ok(value) => value,
                    Err(_) => return Err(failure.unwrap_or_else(|| error(ConfigErrorKind::Parse))),
                }
            }
            Some(b'[') => {
                let mut failure = None;
                let mut deserializer = serde_json::Deserializer::from_str(text);
                let result = deserializer.deserialize_seq(ArrayVisitor {
                    conversion: self,
                    depth,
                    path: path.clone(),
                    failure: &mut failure,
                });
                match result {
                    Ok(value) => value,
                    Err(_) => return Err(failure.unwrap_or_else(|| error(ConfigErrorKind::Parse))),
                }
            }
            Some(b'"') => LayerValue::string(
                serde_json::from_str::<String>(text).map_err(|_| error(ConfigErrorKind::Parse))?,
            ),
            Some(b'n') => LayerValue::null(),
            Some(b't') => LayerValue::boolean(true),
            Some(b'f') => LayerValue::boolean(false),
            _ if text.contains(['.', 'e', 'E']) => {
                // 只有显式小数或指数语法进入浮点分支，其余数字直接按整数范围检查。
                let number = text
                    .parse::<f64>()
                    .map_err(|_| error(ConfigErrorKind::UnsupportedValue))?;
                LayerValue::float(number).map_err(|_| error(ConfigErrorKind::UnsupportedValue))?
            }
            Some(b'-') => LayerValue::integer(
                text.parse::<i64>()
                    .map_err(|_| error(ConfigErrorKind::UnsupportedValue))?,
            ),
            _ => LayerValue::unsigned(
                text.parse::<u64>()
                    .map_err(|_| error(ConfigErrorKind::UnsupportedValue))?,
            ),
        };
        Ok(value.with_origin(origin))
    }
}

/// 逐项读取对象，不先落入会覆盖同名键的 Map。
///
/// 键先按 JSON 转义规则解码，因此 `a` 与 `\u0061` 也会被识别为同一键。
struct ObjectVisitor<'a, 'text> {
    conversion: &'a mut Conversion<'text>,
    depth: usize,
    path: ConfigPath,
    origin: Origin,
    failure: &'a mut Option<ConfigError>,
}

impl<'text> Visitor<'text> for ObjectVisitor<'_, 'text> {
    type Value = LayerValue;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an object")
    }

    fn visit_map<A>(self, mut access: A) -> Result<LayerValue, A::Error>
    where
        A: MapAccess<'text>,
    {
        let mut keys = HashSet::new();
        let mut fields = Vec::new();
        while let Some(key) = access.next_key::<String>()? {
            // 在读取和接受第二个值前拒绝重复声明，不能使用后者静默覆盖前者。
            if !keys.insert(key.clone()) {
                *self.failure = Some(
                    ConfigError::new(ConfigErrorKind::DuplicateKey)
                        .at_path(self.path.clone())
                        .with_origin(self.origin.clone()),
                );
                return Err(A::Error::custom("duplicate configuration key"));
            }
            let raw = access.next_value::<&RawValue>()?;
            let value = match self.conversion.convert(
                raw,
                self.depth + 1,
                self.path.clone().key(key.clone()),
            ) {
                Ok(value) => value,
                Err(error) => {
                    *self.failure = Some(error);
                    return Err(A::Error::custom("invalid configuration value"));
                }
            };
            fields.push((key, value));
        }
        LayerValue::object(fields).map_err(|error| {
            *self.failure = Some(error);
            A::Error::custom("invalid configuration object")
        })
    }
}

/// 按原顺序转换数组元素，使用真实索引定位错误，不跳过失败的元素。
struct ArrayVisitor<'a, 'text> {
    conversion: &'a mut Conversion<'text>,
    depth: usize,
    path: ConfigPath,
    failure: &'a mut Option<ConfigError>,
}

impl<'text> Visitor<'text> for ArrayVisitor<'_, 'text> {
    type Value = LayerValue;

    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("an array")
    }

    fn visit_seq<A>(self, mut access: A) -> Result<LayerValue, A::Error>
    where
        A: SeqAccess<'text>,
    {
        let mut values = Vec::new();
        while let Some(raw) = access.next_element::<&RawValue>()? {
            let value = match self.conversion.convert(
                raw,
                self.depth + 1,
                self.path.clone().index(values.len()),
            ) {
                Ok(value) => value,
                Err(error) => {
                    *self.failure = Some(error);
                    return Err(A::Error::custom("invalid configuration value"));
                }
            };
            values.push(value);
        }
        Ok(LayerValue::array(values))
    }
}
