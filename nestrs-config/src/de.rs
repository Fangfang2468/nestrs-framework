// 将已经加载、合并完成的不可变节点树绑定为调用者请求的 Rust 类型。
//
// 本模块不读取文件或环境变量，也不负责配置覆盖规则。目标类型的 Deserialize
// 决定需要哪种数据形态，Decoder 将对应节点交给 Serde Visitor；序列、对象和枚举
// 再通过各自的访问器逐层绑定。默认值、字段别名和未知字段策略由目标类型的
// Deserialize 实现负责，本模块不复制 Serde derive 的规则。
//
// String 表示来源已确定为字符串的值，Text 表示环境变量等尚未指定目标类型的文本。
// 只有明确的标量请求才能将 Text 转成数字或布尔值，不能先猜测类型再丢失原始文本。
// 所有绑定错误先经过私有 BindError 与 Context 安全边界，最后统一转换为公共错误；
// 失败值、动态 Map 键和自定义错误消息不得借由诊断或来源信息泄漏。

use std::cell::Cell;
use std::collections::btree_map;
use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};

use crate::model::{Node, Value};
use crate::{ConfigError, ConfigErrorKind, ConfigLocation, ConfigPath, MapRole, Origin};

// 绑定的唯一入口。DeserializeOwned 保证交付结果不借用本次解码上下文或配置节点，
// 内部仍可把节点里的字符串临时借给 Visitor，避免提前复制整棵树。
pub(crate) fn decode<T: DeserializeOwned>(
    node: &Node,
    path: &ConfigPath,
) -> Result<T, ConfigError> {
    // 自定义 Deserialize 可能先成功读完整个 Map，再返回对象级错误。所有嵌套适配器
    // 共享栈上的标记，确保外层补充诊断时也不会恢复可能包含动态键的来源。
    let suppress_fallback_origin = Cell::new(false);
    let decoder = Decoder::new(node, ConfigLocation::from(path), &suppress_fallback_origin);
    decoder
        .context
        .finish(T::deserialize(decoder.clone()))
        .map_err(|error| {
            let mut result = ConfigError::new(error.kind);
            if let Some(location) = error.location {
                result = result.at_location(location);
            }
            if let Some(origin) = error.origin {
                result = result.with_origin(origin);
            }
            result
        })
}

// Serde 的错误参数可能通过 Display 暴露被拒绝的值或用户自定义文本，Expected 和
// Unexpected 也不例外。这里仅保留错误类别及受控位置，不格式化、不保存这些参数。
#[derive(Debug)]
struct BindError {
    // 对外可稳定识别的失败类别，不携带原始解析器消息。
    kind: ConfigErrorKind,
    // 最先确定的安全位置由内层填写，外层只补充缺失信息。
    location: Option<ConfigLocation>,
    // 仅在能安全归属单一来源时保存；动态 Map 路径会清除此项。
    origin: Option<Origin>,
}

impl BindError {
    fn new(kind: ConfigErrorKind) -> Self {
        Self {
            kind,
            location: None,
            origin: None,
        }
    }
}

impl fmt::Display for BindError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("configuration binding failed")
    }
}

impl std::error::Error for BindError {}

impl de::Error for BindError {
    // 连 Display 都不能调用：除了泄漏内容，自定义格式化实现还可能具有副作用或 panic。
    fn custom<T: fmt::Display>(_message: T) -> Self {
        Self::new(ConfigErrorKind::Binding)
    }

    fn invalid_type(_unexpected: de::Unexpected<'_>, _expected: &dyn de::Expected) -> Self {
        Self::new(ConfigErrorKind::TypeMismatch)
    }

    fn invalid_value(_unexpected: de::Unexpected<'_>, _expected: &dyn de::Expected) -> Self {
        Self::new(ConfigErrorKind::TypeMismatch)
    }

    fn invalid_length(_length: usize, _expected: &dyn de::Expected) -> Self {
        Self::new(ConfigErrorKind::TypeMismatch)
    }

    fn unknown_variant(_variant: &str, _expected: &'static [&'static str]) -> Self {
        Self::new(ConfigErrorKind::TypeMismatch)
    }

    fn unknown_field(_field: &str, _expected: &'static [&'static str]) -> Self {
        Self::new(ConfigErrorKind::Binding)
    }

    fn missing_field(_field: &'static str) -> Self {
        Self::new(ConfigErrorKind::Binding)
    }

    fn duplicate_field(_field: &'static str) -> Self {
        Self::new(ConfigErrorKind::Binding)
    }
}

type BindResult<T> = Result<T, BindError>;

// 当前节点的诊断上下文。克隆只复制位置与来源并共享抑制标记，不复制配置值；
// 标记的生命周期限定在单次 decode 内，不需要线程同步、全局状态或额外堆分配。
#[derive(Clone)]
struct Context<'context> {
    // 已知结构体字段可用名称定位；动态 Map 使用条目序号，数组使用下标。
    location: ConfigLocation,
    // 合并对象可能没有唯一来源，此时保留 None，不能猜测某个子节点作为来源。
    origin: Option<Origin>,
    // 单向由 false 变为 true；避免内层 Map 完成后外层恢复不安全来源。
    suppress_fallback_origin: &'context Cell<bool>,
}

impl<'context> Context<'context> {
    fn new(
        node: &Node,
        location: ConfigLocation,
        suppress_fallback_origin: &'context Cell<bool>,
    ) -> Self {
        // 环境变量来源名本身也可能含动态键，因此仅遮住诊断路径还不够。
        let origin = if location.has_entries() {
            None
        } else {
            node.origins.single().cloned()
        };
        Self {
            location,
            origin,
            suppress_fallback_origin,
        }
    }

    fn finish<T>(&self, result: BindResult<T>) -> BindResult<T> {
        result.map_err(|mut error| {
            // 保留内层已经定位的失败，避免逐层传播时退回父对象或根路径。
            if error.location.is_none() {
                error.location = Some(self.location.clone());
                error.origin = self.origin.clone();
            }
            // 保守处理整次绑定：一旦经过动态 Map，后续错误的来源也可缺省。
            // 这覆盖“读完 Map 后才 custom 报错”的情况，此时位置不一定含条目序号。
            if self.suppress_fallback_origin.get()
                || error
                    .location
                    .as_ref()
                    .is_some_and(|path| path.has_entries())
            {
                error.origin = None;
            }
            error
        })
    }

    fn mismatch<T>(&self) -> BindResult<T> {
        self.finish(Err(BindError::new(ConfigErrorKind::TypeMismatch)))
    }

    fn unsupported<T>(&self) -> BindResult<T> {
        self.finish(Err(BindError::new(ConfigErrorKind::UnsupportedValue)))
    }
}

// 单个节点上的 Serde Deserializer。Visitor 请求类型而非源码文本决定转换路线；
// Context 与节点使用同一借用期，使嵌套访问器不能逃出本次读取。
#[derive(Clone)]
struct Decoder<'de> {
    // 已通过加载阶段资源限制与值域检查的节点，解码期间保持只读。
    node: &'de Node,
    context: Context<'de>,
}

impl<'de> Decoder<'de> {
    fn new(
        node: &'de Node,
        location: ConfigLocation,
        suppress_fallback_origin: &'de Cell<bool>,
    ) -> Self {
        Self {
            node,
            context: Context::new(node, location, suppress_fallback_origin),
        }
    }

    // tuple/定长数组可传入长度约束，普通序列则逐项读取。Visitor 成功仍须消费完
    // 序列，不能把多余元素静默丢弃；明确的 IgnoredAny 另有专用处理。
    fn sequence<V: Visitor<'de>>(
        self,
        visitor: V,
        expected_len: Option<usize>,
    ) -> BindResult<V::Value> {
        let Value::Array(values) = &self.node.value else {
            return self.context.mismatch();
        };
        if expected_len.is_some_and(|expected| expected != values.len()) {
            return self.context.mismatch();
        }
        let mut access = ArrayAccess {
            values,
            index: 0,
            context: self.context.clone(),
        };
        let value = self.context.finish(visitor.visit_seq(&mut access))?;
        if access.index != values.len() {
            return self.context.mismatch();
        }
        Ok(value)
    }

    // 同一对象节点既可绑定 struct，也可绑定动态 Map；fields 区分是否存在由
    // Deserialize 提供的静态字段集合，进而决定错误能否安全携带字段名称。
    fn object<V: Visitor<'de>>(
        self,
        visitor: V,
        fields: Option<&'static [&'static str]>,
    ) -> BindResult<V::Value> {
        if fields.is_none() {
            // 在形状检查之前抑制来源：即使 Map 请求遇到非对象输入，也不恢复
            // 可能编码动态键的环境变量来源名。
            self.context.suppress_fallback_origin.set(true);
        }
        let Value::Object(values) = &self.node.value else {
            return self.context.mismatch();
        };
        let mut access = ObjectAccess {
            entries: values.iter(),
            pending: None,
            ordinal: 0,
            fields,
            context: self.context.clone(),
        };
        let value = self.context.finish(visitor.visit_map(&mut access))?;
        // Visitor 不能留下只读取了键、尚未读取值的条目，也不能跳过剩余条目。
        if access.pending.is_some() || access.entries.len() != 0 {
            return self.context.mismatch();
        }
        Ok(value)
    }
}

// 所有整数目标共用同一边界：整数节点通过 TryFrom 检查符号和位宽，Text 直接
// 按目标整数解析。整个过程不经过 f64，避免大整数舍入；浮点节点即使没有小数
// 部分也不自动截断为整数，已确定的 String 同样不参与隐式数字转换。
macro_rules! deserialize_integer {
    ($method:ident, $visit:ident, $integer:ty) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
            let value: $integer = match &self.node.value {
                Value::I64(value) => match <$integer>::try_from(*value) {
                    Ok(value) => value,
                    Err(_) => return self.context.mismatch(),
                },
                Value::U64(value) => match <$integer>::try_from(*value) {
                    Ok(value) => value,
                    Err(_) => return self.context.mismatch(),
                },
                Value::Text(value) => match value.parse::<$integer>() {
                    Ok(value) => value,
                    Err(_) => return self.context.mismatch(),
                },
                _ => return self.context.mismatch(),
            };
            self.context.finish(visitor.$visit(value))
        }
    };
}

impl<'de> de::Deserializer<'de> for Decoder<'de> {
    type Error = BindError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        // 调用者尚未指定目标类型时保留来源形态。flatten/untagged 可能先通过
        // 此入口缓存 Serde 内容，因此缓存后的 Text 仍是字符串，不承诺后续再次
        // 经过本模块的文本转数字路径。
        let result = match &self.node.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(value) => visitor.visit_bool(*value),
            Value::I64(value) => visitor.visit_i64(*value),
            Value::U64(value) => visitor.visit_u64(*value),
            Value::F64(value) if value.is_finite() => visitor.visit_f64(*value),
            Value::F64(_) => return self.context.unsupported(),
            Value::String(value) | Value::Text(value) => visitor.visit_borrowed_str(value),
            Value::Array(_) => return self.sequence(visitor, None),
            Value::Object(_) => return self.object(visitor, None),
        };
        self.context.finish(result)
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        // 文本布尔只接受精确的小写 true/false，不扩展 yes、1 或去空白等规则。
        let value = match &self.node.value {
            Value::Bool(value) => *value,
            Value::Text(value) if value == "true" => true,
            Value::Text(value) if value == "false" => false,
            _ => return self.context.mismatch(),
        };
        self.context.finish(visitor.visit_bool(value))
    }

    deserialize_integer!(deserialize_i8, visit_i8, i8);
    deserialize_integer!(deserialize_i16, visit_i16, i16);
    deserialize_integer!(deserialize_i32, visit_i32, i32);
    deserialize_integer!(deserialize_i64, visit_i64, i64);
    deserialize_integer!(deserialize_i128, visit_i128, i128);
    deserialize_integer!(deserialize_u8, visit_u8, u8);
    deserialize_integer!(deserialize_u16, visit_u16, u16);
    deserialize_integer!(deserialize_u32, visit_u32, u32);
    deserialize_integer!(deserialize_u64, visit_u64, u64);
    deserialize_integer!(deserialize_u128, visit_u128, u128);

    // 浮点目标允许正常的浮点精度舍入，但收窄至 f32 后必须再次检查有限性，
    // 防止合法 f64 转成无穷大；Text 直接按 f32 解析，不先经 f64 二次舍入。
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        let value = match &self.node.value {
            Value::I64(value) => *value as f32,
            Value::U64(value) => *value as f32,
            Value::F64(value) => *value as f32,
            Value::Text(value) => match value.parse::<f32>() {
                Ok(value) => value,
                Err(_) => return self.context.mismatch(),
            },
            _ => return self.context.mismatch(),
        };
        if !value.is_finite() {
            return self.context.unsupported();
        }
        self.context.finish(visitor.visit_f32(value))
    }

    // NaN、无穷大以及文本指数溢出均不属于支持的配置值域，统一返回
    // UnsupportedValue；语法本身无法解析则归为 TypeMismatch。
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        let value = match &self.node.value {
            Value::I64(value) => *value as f64,
            Value::U64(value) => *value as f64,
            Value::F64(value) => *value,
            Value::Text(value) => match value.parse::<f64>() {
                Ok(value) => value,
                Err(_) => return self.context.mismatch(),
            },
            _ => return self.context.mismatch(),
        };
        if !value.is_finite() {
            return self.context.unsupported();
        }
        self.context.finish(visitor.visit_f64(value))
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        // Rust char 是一个 Unicode 标量值，不能用 UTF-8 字节长度是否为 1 判断。
        let text = match &self.node.value {
            Value::String(text) | Value::Text(text) => text,
            _ => return self.context.mismatch(),
        };
        let mut chars = text.chars();
        let Some(value) = chars.next() else {
            return self.context.mismatch();
        };
        if chars.next().is_some() {
            return self.context.mismatch();
        }
        self.context.finish(visitor.visit_char(value))
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        match &self.node.value {
            Value::String(value) | Value::Text(value) => {
                self.context.finish(visitor.visit_borrowed_str(value))
            }
            _ => self.context.mismatch(),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.deserialize_str(visitor)
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        // 字符串按现有 UTF-8 字节交付，不推测 Base64 等编码；数组仍逐项按
        // Visitor 请求的字节类型绑定，因此超出 u8 范围的元素会正常失败。
        match &self.node.value {
            Value::String(value) | Value::Text(value) => self
                .context
                .finish(visitor.visit_borrowed_bytes(value.as_bytes())),
            Value::Array(_) => self.sequence(visitor, None),
            _ => self.context.mismatch(),
        }
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        // 此处只处理已存在节点的 null。路径缺失由 Configuration 在进入解码前
        // 处理，不能与 Option<T> 的 None 混淆；文本 "null" 仍是普通 Text。
        if matches!(&self.node.value, Value::Null) {
            self.context.finish(visitor.visit_none())
        } else {
            self.context.finish(visitor.visit_some(self.clone()))
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        if matches!(&self.node.value, Value::Null) {
            self.context.finish(visitor.visit_unit())
        } else {
            self.context.mismatch()
        }
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> BindResult<V::Value> {
        self.deserialize_unit(visitor)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> BindResult<V::Value> {
        // newtype 不额外增加配置层级，并复用原上下文，使包装外的错误仍遵守
        // 内部 Map 已建立的来源抑制规则。
        self.context
            .finish(visitor.visit_newtype_struct(self.clone()))
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.sequence(visitor, None)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> BindResult<V::Value> {
        self.sequence(visitor, Some(len))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> BindResult<V::Value> {
        self.sequence(visitor, Some(len))
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.object(visitor, None)
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> BindResult<V::Value> {
        self.object(visitor, Some(fields))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> BindResult<V::Value> {
        // Serde 的外部标签枚举：字符串表示无负载变体，单键对象表示带负载变体。
        // 内部标签、相邻标签及 untagged 的编排由目标 Deserialize 使用其他入口完成。
        let (variant, payload, context) = match &self.node.value {
            Value::String(variant) | Value::Text(variant) => {
                (variant.as_str(), None, self.context.clone())
            }
            Value::Object(values) if values.len() == 1 => {
                let Some((variant, payload)) = values.iter().next() else {
                    return self.context.mismatch();
                };
                // 仅静态 variants 集合中的名称可进入诊断；未知标签属于配置输入，
                // 用条目位置代替，避免错误信息包含用户提供的标签内容。
                let location = if let Some(known) = variants.iter().find(|known| **known == variant)
                {
                    self.context.location.clone().field(*known)
                } else {
                    self.context.location.clone().entry(0, MapRole::Key)
                };
                (
                    variant.as_str(),
                    Some(payload),
                    Context::new(payload, location, self.context.suppress_fallback_origin),
                )
            }
            _ => return self.context.mismatch(),
        };
        self.context.finish(visitor.visit_enum(TaggedEnum {
            variant,
            payload,
            context,
        }))
    }

    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        // 节点树已在加载时完成解析和资源限制检查。忽略一个值不需要再次递归
        // 遍历其子树，也不会留下尚未解析的输入文本。
        self.context.finish(visitor.visit_unit())
    }
}

// 将数组按 Serde 的种子接口逐项交付。种子负责选定元素的 Deserialize 行为，
// 访问器只推进下标、传递子节点并维护精确的错误位置。
struct ArrayAccess<'de> {
    values: &'de [Node],
    // 下一次读取的元素下标，同时用于验证 Visitor 是否消费完数组。
    index: usize,
    context: Context<'de>,
}

impl<'de> SeqAccess<'de> for ArrayAccess<'de> {
    type Error = BindError;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> BindResult<Option<T::Value>> {
        let Some(node) = self.values.get(self.index) else {
            return Ok(None);
        };
        let location = self.context.location.clone().index(self.index);
        self.index += 1;
        let decoder = Decoder::new(node, location, self.context.suppress_fallback_origin);
        decoder
            .context
            .finish(seed.deserialize(decoder.clone()))
            .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.values.len() - self.index)
    }
}

// 对象共享一个顺序访问器。BTreeMap 的稳定遍历使同一配置的错误条目序号稳定；
// 该序号是诊断坐标，不是可以重新查询配置的路径或动态键的替代存储。
struct ObjectAccess<'de> {
    entries: btree_map::Iter<'de, String, Node>,
    // Serde 分开请求 key/value：读取 key 后暂存同一条目，读取 value 时取走，
    // 防止重复请求键、跳过值或把另一个条目的值错误配对。
    pending: Option<(&'de str, &'de Node, usize)>,
    // 从零开始累计已经选中的条目，与动态键的文本内容无关。
    ordinal: usize,
    // struct 的静态字段白名单；动态 Map 和未知字段均使用条目序号定位。
    fields: Option<&'static [&'static str]>,
    context: Context<'de>,
}

impl ObjectAccess<'_> {
    fn location(&self, key: &str, ordinal: usize, role: MapRole) -> ConfigLocation {
        // 选择白名单中的静态名称，不把未经确认的输入键直接拼进错误链。
        if let Some(known) = self
            .fields
            .and_then(|fields| fields.iter().find(|known| **known == key))
        {
            self.context.location.clone().field(*known)
        } else {
            self.context.location.clone().entry(ordinal, role)
        }
    }
}

impl<'de> MapAccess<'de> for ObjectAccess<'de> {
    type Error = BindError;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> BindResult<Option<K::Value>> {
        if self.pending.is_some() {
            return self.context.mismatch();
        }
        let Some((key, node)) = self.entries.next() else {
            return Ok(None);
        };
        let ordinal = self.ordinal;
        self.ordinal += 1;
        self.pending = Some((key, node, ordinal));
        let context = Context::new(
            node,
            self.location(key, ordinal, MapRole::Key),
            self.context.suppress_fallback_origin,
        );
        context
            .finish(seed.deserialize(KeyDecoder {
                text: key,
                context: context.clone(),
            }))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> BindResult<V::Value> {
        let Some((key, node, ordinal)) = self.pending.take() else {
            return self.context.mismatch();
        };
        let decoder = Decoder::new(
            node,
            self.location(key, ordinal, MapRole::Value),
            self.context.suppress_fallback_origin,
        );
        decoder.context.finish(seed.deserialize(decoder.clone()))
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len() + usize::from(self.pending.is_some()))
    }
}

// 对象键的专用解码器。节点模型的键始终是字符串，但目标 Map 可明确请求整数、
// 布尔、字符或无负载枚举键；这是目标类型驱动的键转换，不改变对应 value 的类型。
#[derive(Clone)]
struct KeyDecoder<'de> {
    // 只借用于成功绑定，失败时不会把该文本写入 BindError。
    text: &'de str,
    context: Context<'de>,
}

// 直接按目标数值类型解析键，保持整数范围检查；浮点键另行检查有限性。
macro_rules! deserialize_key_number {
    ($method:ident, $visit:ident, $number:ty) => {
        fn $method<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
            let Ok(value) = self.text.parse::<$number>() else {
                return self.context.mismatch();
            };
            self.context.finish(visitor.$visit(value))
        }
    };
}

impl<'de> de::Deserializer<'de> for KeyDecoder<'de> {
    type Error = BindError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.context.finish(visitor.visit_borrowed_str(self.text))
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        let value = match self.text {
            "true" => true,
            "false" => false,
            _ => return self.context.mismatch(),
        };
        self.context.finish(visitor.visit_bool(value))
    }

    deserialize_key_number!(deserialize_i8, visit_i8, i8);
    deserialize_key_number!(deserialize_i16, visit_i16, i16);
    deserialize_key_number!(deserialize_i32, visit_i32, i32);
    deserialize_key_number!(deserialize_i64, visit_i64, i64);
    deserialize_key_number!(deserialize_i128, visit_i128, i128);
    deserialize_key_number!(deserialize_u8, visit_u8, u8);
    deserialize_key_number!(deserialize_u16, visit_u16, u16);
    deserialize_key_number!(deserialize_u32, visit_u32, u32);
    deserialize_key_number!(deserialize_u64, visit_u64, u64);
    deserialize_key_number!(deserialize_u128, visit_u128, u128);

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        let Ok(value) = self.text.parse::<f32>() else {
            return self.context.mismatch();
        };
        if !value.is_finite() {
            return self.context.unsupported();
        }
        self.context.finish(visitor.visit_f32(value))
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        let Ok(value) = self.text.parse::<f64>() else {
            return self.context.mismatch();
        };
        if !value.is_finite() {
            return self.context.unsupported();
        }
        self.context.finish(visitor.visit_f64(value))
    }

    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        let mut chars = self.text.chars();
        let Some(value) = chars.next() else {
            return self.context.mismatch();
        };
        if chars.next().is_some() {
            return self.context.mismatch();
        }
        self.context.finish(visitor.visit_char(value))
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> BindResult<V::Value> {
        self.context.finish(visitor.visit_some(self.clone()))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> BindResult<V::Value> {
        self.context
            .finish(visitor.visit_newtype_struct(self.clone()))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> BindResult<V::Value> {
        self.context.finish(visitor.visit_enum(TaggedEnum {
            variant: self.text,
            payload: None,
            context: self.context.clone(),
        }))
    }

    // 没有专用转换的键类型仍获得字符串 Visitor 输入，由该类型自行接受或拒绝；
    // 不将字符串键伪装成对象、序列或复合枚举负载。
    serde::forward_to_deserialize_any! {
        str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct identifier ignored_any
    }
}

// 枚举读取分为“选择变体”和“读取负载”两阶段，先让目标类型识别合法变体，
// 再交付对应的 EnumPayload，避免在尚未确定变体时猜测负载结构。
struct TaggedEnum<'de> {
    variant: &'de str,
    // 字符串形式没有负载；单键对象形式保留其子节点。
    payload: Option<&'de Node>,
    context: Context<'de>,
}

impl<'de> EnumAccess<'de> for TaggedEnum<'de> {
    type Error = BindError;
    type Variant = EnumPayload<'de>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> BindResult<(V::Value, Self::Variant)> {
        let variant = self.context.finish(seed.deserialize(KeyDecoder {
            text: self.variant,
            context: self.context.clone(),
        }))?;
        Ok((
            variant,
            EnumPayload {
                node: self.payload,
                context: self.context,
            },
        ))
    }
}

// 已选枚举变体的负载访问器，复用普通节点的序列/对象绑定和安全诊断规则。
struct EnumPayload<'de> {
    node: Option<&'de Node>,
    context: Context<'de>,
}

impl<'de> VariantAccess<'de> for EnumPayload<'de> {
    type Error = BindError;

    fn unit_variant(self) -> BindResult<()> {
        // 无负载变体可写作字符串，也可写作值为 null 的单键对象；其他值不吞掉。
        match self.node {
            None => Ok(()),
            Some(node) if matches!(&node.value, Value::Null) => Ok(()),
            Some(_) => self.context.mismatch(),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> BindResult<T::Value> {
        let Some(node) = self.node else {
            return self.context.mismatch();
        };
        self.context.finish(seed.deserialize(Decoder {
            node,
            context: self.context.clone(),
        }))
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> BindResult<V::Value> {
        let Some(node) = self.node else {
            return self.context.mismatch();
        };
        Decoder {
            node,
            context: self.context,
        }
        .sequence(visitor, Some(len))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> BindResult<V::Value> {
        let Some(node) = self.node else {
            return self.context.mismatch();
        };
        Decoder {
            node,
            context: self.context,
        }
        .object(visitor, Some(fields))
    }
}
