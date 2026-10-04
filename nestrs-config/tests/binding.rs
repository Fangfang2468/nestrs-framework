// 通过公开的 Memory -> Configuration -> get/get_required 路径验证绑定契约，
// 不直接访问私有 Decoder。这样既覆盖 Serde 适配，也覆盖真实客户端交付结果和
// 错误的边界；测试数据在内存中构造，避免文件解析器差异干扰类型绑定场景。
//
// 测试重点包括来源值形态、目标类型约束、Serde 数据模型和诊断安全。错误检查
// 同时覆盖 Display、Debug 与 source 链，防止顶层消息打码而下层仍保留敏感内容。

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use nestrs_config::sources::Memory;
use nestrs_config::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigLocation, ConfigPath, Configuration,
    LayerValue, MapRole, Origin,
};
use serde::de::{self, DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

// 保留 LayerValue 的显式类型，不经 JSON 等中间格式隐式改写数字或文本。
fn object(fields: Vec<(&str, LayerValue)>) -> LayerValue {
    LayerValue::object(
        fields
            .into_iter()
            .map(|(name, value)| (name.into(), value))
            .collect(),
    )
    .unwrap()
}

// 所有样本使用相同的安全来源名；需要验证来源覆盖的测试再给具体节点指定来源。
fn configuration(fields: Vec<(&str, LayerValue)>) -> Configuration {
    Configuration::builder()
        .add_source(Memory::new(
            ConfigLayer::from_value(object(fields), Origin::named("binding-test")).unwrap(),
        ))
        .build()
        .unwrap()
}

// 固定公开查询路径，便于在不同类型场景中比较诊断位置而不泄漏测试值作为路径。
fn read<T: DeserializeOwned>(value: LayerValue) -> Result<T, ConfigError> {
    configuration(vec![("value", value)]).get_required("value")
}

// 检查完整可见错误链，不能只以顶层 Display 不含敏感文本就判定安全。
fn assert_safe(error: &ConfigError, secret: &str) {
    assert!(!error.to_string().contains(secret));
    assert!(!format!("{error:?}").contains(secret));
    let mut cause = error.source();
    while let Some(error) = cause {
        assert!(!error.to_string().contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        cause = error.source();
    }
}

// 同一文本由请求的标量类型解释；已确定的 String 不转换，String 目标保留前导零。
// 同时限定布尔文本和整数文本语法，不悄悄接受大小写、空白或其他宽松写法。
#[test]
fn text_converts_only_for_the_requested_scalar_type() {
    assert_eq!(read::<u16>(LayerValue::text("5432")).unwrap(), 5432);
    assert_eq!(
        read::<String>(LayerValue::text("005432")).unwrap(),
        "005432"
    );
    assert_eq!(read::<String>(LayerValue::string("5432")).unwrap(), "5432");
    assert_eq!(
        read::<u16>(LayerValue::string("5432")).unwrap_err().kind(),
        ConfigErrorKind::TypeMismatch
    );
    assert!(read::<String>(LayerValue::unsigned(5432)).is_err());

    assert!(read::<bool>(LayerValue::text("true")).unwrap());
    assert!(!read::<bool>(LayerValue::text("false")).unwrap());
    for text in ["True", "TRUE", "1", "yes", " true", "false "] {
        assert!(read::<bool>(LayerValue::text(text)).is_err(), "{text}");
    }
    for text in [" 42", "42 ", "1_000", "42.0", "1e2", ""] {
        assert!(read::<u16>(LayerValue::text(text)).is_err(), "{text}");
    }
}

// 极限整数必须保持精确；有符号/无符号与窄位宽转换不能环绕或截断。
// 浮点数即使数值上恰好是整数，也不成为整数绑定的隐式输入。
#[test]
fn integer_binding_is_exact_and_checks_target_ranges() {
    assert_eq!(
        read::<i64>(LayerValue::integer(i64::MIN)).unwrap(),
        i64::MIN
    );
    assert_eq!(
        read::<u64>(LayerValue::unsigned(u64::MAX)).unwrap(),
        u64::MAX
    );
    assert_eq!(
        read::<u64>(LayerValue::text(u64::MAX.to_string())).unwrap(),
        u64::MAX
    );
    assert_eq!(
        read::<i128>(LayerValue::text(i128::MIN.to_string())).unwrap(),
        i128::MIN
    );
    assert_eq!(
        read::<u128>(LayerValue::text(u128::MAX.to_string())).unwrap(),
        u128::MAX
    );
    assert_eq!(
        read::<i128>(LayerValue::unsigned(u64::MAX)).unwrap(),
        i128::from(u64::MAX)
    );
    assert!(read::<u64>(LayerValue::integer(-1)).is_err());
    assert!(read::<i64>(LayerValue::unsigned(u64::MAX)).is_err());
    assert!(read::<u8>(LayerValue::unsigned(256)).is_err());
    assert!(read::<i8>(LayerValue::integer(-129)).is_err());
    assert!(read::<u16>(LayerValue::float(42.0).unwrap()).is_err());
    assert!(read::<u64>(LayerValue::float(42.9).unwrap()).is_err());
}

// 合法浮点值正常交付，NaN、无穷大、指数溢出和 f64 -> f32 溢出统一拒绝，
// 确认“不支持的值域”与普通类型不匹配能够通过错误类别区分。
#[test]
fn floating_binding_rejects_nonfinite_and_narrowing_overflow() {
    assert_eq!(read::<f64>(LayerValue::text("1.25e2")).unwrap(), 125.0);
    assert_eq!(read::<f32>(LayerValue::integer(-3)).unwrap(), -3.0);
    assert_eq!(read::<f32>(LayerValue::float(1.5).unwrap()).unwrap(), 1.5);
    for text in ["NaN", "inf", "-inf", "1e999"] {
        assert_eq!(
            read::<f64>(LayerValue::text(text)).unwrap_err().kind(),
            ConfigErrorKind::UnsupportedValue
        );
    }
    assert_eq!(
        read::<f32>(LayerValue::float(f64::MAX).unwrap())
            .unwrap_err()
            .kind(),
        ConfigErrorKind::UnsupportedValue
    );
    assert_eq!(
        read::<f32>(LayerValue::text("1e100")).unwrap_err().kind(),
        ConfigErrorKind::UnsupportedValue
    );
}

// get 的外层 Option 表示路径存在性，目标 Option 表示已存在值是否为 null。
// 文字 null 不自动解释为空值，get_required 也不能因目标允许 None 而容忍缺失路径。
#[test]
fn null_missing_and_text_null_remain_distinct() {
    let config = configuration(vec![("present", LayerValue::null())]);
    assert_eq!(config.get::<Option<u16>>("present").unwrap(), Some(None));
    assert_eq!(config.get::<Option<u16>>("absent").unwrap(), None);
    assert_eq!(config.get_required::<Option<u16>>("present").unwrap(), None);
    assert_eq!(
        config
            .get_required::<Option<u16>>("absent")
            .unwrap_err()
            .kind(),
        ConfigErrorKind::Missing
    );
    assert_eq!(
        read::<Option<String>>(LayerValue::text("null")).unwrap(),
        Some("null".into())
    );
    assert!(read::<Option<u16>>(LayerValue::text("null")).is_err());
    assert!(read::<u16>(LayerValue::null()).is_err());
}

// 使用显式默认函数，验证默认值来自目标 Deserialize，而不是写入配置树。
fn default_pool_size() -> u16 {
    16
}

// 普通 Serde 模型组合主键名、输入别名、缺省值、可选字段和未知字段拒绝策略，
// 不依赖 Nestrs 配置宏或额外校验机制。
#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Database {
    host: String,
    #[serde(
        rename = "pool-size",
        alias = "pool_size",
        default = "default_pool_size"
    )]
    pool_size: u16,
    replica: Option<String>,
}

// 重命名和别名都能绑定同一字段；同时出现则按重复字段拒绝。
// 默认值仅弥补缺失，不能修复显式 null、错误类型或另一个必填字段的缺失。
#[test]
fn ordinary_serde_mapping_defaults_and_strictness_are_respected() {
    let db: Database = read(object(vec![("host", LayerValue::string("localhost"))])).unwrap();
    assert_eq!(
        db,
        Database {
            host: "localhost".into(),
            pool_size: 16,
            replica: None
        }
    );
    for label in ["pool-size", "pool_size"] {
        let db: Database = read(object(vec![
            ("host", LayerValue::string("localhost")),
            (label, LayerValue::unsigned(32)),
        ]))
        .unwrap();
        assert_eq!(db.pool_size, 32);
    }
    for bad in [LayerValue::null(), LayerValue::string("bad")] {
        assert!(
            read::<Database>(object(vec![
                ("host", LayerValue::string("localhost")),
                ("pool-size", bad),
            ]))
            .is_err()
        );
    }
    assert!(
        read::<Database>(object(vec![
            ("host", LayerValue::string("localhost")),
            ("pool-size", LayerValue::unsigned(16)),
            ("pool_size", LayerValue::unsigned(32)),
        ]))
        .is_err()
    );
    assert!(read::<Database>(object(vec![("pool-size", LayerValue::unsigned(32))])).is_err());
}

// 只有缺省字段的模型，用来区分“存在的空对象”与“对象路径完全不存在”。
#[derive(Debug, Deserialize, PartialEq)]
struct Defaults {
    #[serde(default = "default_pool_size")]
    pool_size: u16,
}

// 字段默认值只出现在绑定结果中，不能回写快照，也不能凭空创建缺失的配置分组。
#[test]
fn field_defaults_apply_to_present_empty_objects_without_creating_missing_sections() {
    let config = configuration(vec![("empty", object(vec![]))]);
    assert_eq!(
        config.get_required::<Defaults>("empty").unwrap(),
        Defaults { pool_size: 16 }
    );
    assert!(config.get::<u16>("empty.pool_size").unwrap().is_none());
    assert!(config.get::<Defaults>("absent").unwrap().is_none());
    assert_eq!(
        config
            .get_required::<Defaults>("absent")
            .unwrap_err()
            .kind(),
        ConfigErrorKind::Missing
    );
}

// 三种不同的 Serde 结构形态：透明单字段包装、定长位置字段和无字段单元结构体。
#[derive(Debug, Deserialize, PartialEq)]
struct Port(u16);

#[derive(Debug, Deserialize, PartialEq)]
struct Pair(String, bool);

#[derive(Debug, Deserialize, PartialEq)]
struct Unit;

// 验证非对象模型也按 Serde 语义绑定，固定长度必须匹配；字符按 Unicode 标量
// 而非 UTF-8 字节数判断，因此一个中文字符合法，空字符串和两个字符非法。
#[test]
fn binding_covers_newtypes_tuples_arrays_units_and_unicode_chars() {
    assert_eq!(read::<Port>(LayerValue::text("8080")).unwrap(), Port(8080));
    assert_eq!(
        read::<Pair>(LayerValue::array(vec![
            LayerValue::string("host"),
            LayerValue::boolean(true)
        ]))
        .unwrap(),
        Pair("host".into(), true)
    );
    assert_eq!(
        read::<(u16, bool)>(LayerValue::array(vec![
            LayerValue::unsigned(42),
            LayerValue::text("true")
        ]))
        .unwrap(),
        (42, true)
    );
    assert_eq!(
        read::<[u8; 2]>(LayerValue::array(vec![
            LayerValue::unsigned(1),
            LayerValue::unsigned(2)
        ]))
        .unwrap(),
        [1, 2]
    );
    assert!(read::<[u8; 2]>(LayerValue::array(vec![LayerValue::unsigned(1)])).is_err());
    assert!(
        read::<(u8,)>(LayerValue::array(vec![
            LayerValue::unsigned(1),
            LayerValue::unsigned(2)
        ]))
        .is_err()
    );
    read::<()>(LayerValue::null()).unwrap();
    assert_eq!(read::<Unit>(LayerValue::null()).unwrap(), Unit);
    assert_eq!(read::<char>(LayerValue::text("配")).unwrap(), '配');
    assert!(read::<char>(LayerValue::text("")).is_err());
    assert!(read::<char>(LayerValue::text("配置")).is_err());
}

// 外部标签枚举覆盖单元、newtype、元组和结构体四种负载；随后两个模型分别验证
// Serde 自己编排的内部标签和相邻标签，不要求解码器识别业务枚举名称。
#[derive(Debug, Deserialize, PartialEq)]
enum Choice {
    Off,
    Port(u16),
    Pair(String, u16),
    Server { host: String, port: u16 },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind")]
enum InternalChoice {
    Server { port: u16 },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data")]
enum AdjacentChoice {
    Port(u16),
}

// 同一底层值模型应支持各类标准枚举表示，同时外部标签的 Text 数字负载仍可按
// 明确的目标类型转换，不因枚举包装丢失普通标量绑定能力。
#[test]
fn serde_enum_representations_bind_through_the_data_model() {
    assert_eq!(
        read::<Choice>(LayerValue::string("Off")).unwrap(),
        Choice::Off
    );
    assert_eq!(
        read::<Choice>(object(vec![("Off", LayerValue::null())])).unwrap(),
        Choice::Off
    );
    assert_eq!(
        read::<Choice>(object(vec![("Port", LayerValue::text("8080"))])).unwrap(),
        Choice::Port(8080)
    );
    assert_eq!(
        read::<Choice>(object(vec![(
            "Pair",
            LayerValue::array(vec![LayerValue::string("host"), LayerValue::unsigned(80)])
        )]))
        .unwrap(),
        Choice::Pair("host".into(), 80)
    );
    assert_eq!(
        read::<Choice>(object(vec![(
            "Server",
            object(vec![
                ("host", LayerValue::string("host")),
                ("port", LayerValue::unsigned(80))
            ])
        )]))
        .unwrap(),
        Choice::Server {
            host: "host".into(),
            port: 80
        }
    );
    assert_eq!(
        read::<InternalChoice>(object(vec![
            ("kind", LayerValue::string("Server")),
            ("port", LayerValue::unsigned(80))
        ]))
        .unwrap(),
        InternalChoice::Server { port: 80 }
    );
    assert_eq!(
        read::<AdjacentChoice>(object(vec![
            ("kind", LayerValue::string("Port")),
            ("data", LayerValue::unsigned(80))
        ]))
        .unwrap(),
        AdjacentChoice::Port(80)
    );
}

// untagged/flatten 会先缓存通用 Serde 内容；这些模型用于观察缓冲前后是否还会
// 进入配置解码器的目标类型转换入口。
#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum NumberOrText {
    Number(u16),
    Text(String),
}

#[derive(Debug, Deserialize, PartialEq)]
struct Numeric {
    port: u16,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Flattened {
    #[serde(flatten)]
    child: Numeric,
}

// 明确记录文本转换的适用边界：直接字段读取可以将 Text 转为数字，但先经
// deserialize_any 缓存的 Text 保持字符串。使用真实数值节点时两种路径均可绑定。
#[test]
fn buffered_serde_content_keeps_text_as_text() {
    assert_eq!(
        read::<NumberOrText>(LayerValue::text("42")).unwrap(),
        NumberOrText::Text("42".into())
    );
    assert_eq!(
        read::<NumberOrText>(LayerValue::unsigned(42)).unwrap(),
        NumberOrText::Number(42)
    );
    assert_eq!(
        read::<Numeric>(object(vec![("port", LayerValue::text("42"))])).unwrap(),
        Numeric { port: 42 }
    );
    assert!(read::<Flattened>(object(vec![("port", LayerValue::text("42"))])).is_err());
    assert_eq!(
        read::<Flattened>(object(vec![("port", LayerValue::unsigned(42))])).unwrap(),
        Flattened {
            child: Numeric { port: 42 }
        }
    );
}

// 对象 -> 数组 -> 对象字段的层级，用于验证错误不会在逐层传播时退回父位置。
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct Servers {
    servers: Vec<Numeric>,
}

// 第二个元素中的 port 故意类型错误：诊断应指向该精确位置，并保留该节点的
// 文件行列来源，拒绝值本身不能进入 Display、Debug 或错误链。
#[test]
fn struct_and_array_errors_keep_the_deepest_safe_location_and_origin() {
    let origin = Origin::file("config.toml").at(9, 7).unwrap();
    let error = read::<Servers>(object(vec![(
        "servers",
        LayerValue::array(vec![
            object(vec![("port", LayerValue::unsigned(80))]),
            object(vec![(
                "port",
                LayerValue::string("not-a-number").with_origin(origin.clone()),
            )]),
        ]),
    )]))
    .unwrap_err();
    assert_eq!(
        error.location(),
        Some(
            &ConfigLocation::from(&ConfigPath::parse("value.servers").unwrap())
                .index(1)
                .field("port")
        )
    );
    assert_eq!(error.origin(), Some(&origin));
    assert_safe(&error, "not-a-number");
}

// 分别触发 Map 键转换失败和 Map 值的嵌套字段失败，要求稳定条目序号与角色。
// 动态键既不能进入 location，也不能借环境变量 origin 再次泄漏。
#[test]
fn dynamic_map_keys_and_origins_never_appear_in_binding_errors() {
    const SECRET: &str = "credential-unique-sentinel";
    let error = read::<BTreeMap<u16, bool>>(object(vec![(
        SECRET,
        LayerValue::boolean(true).with_origin(Origin::environment(format!("APP__{SECRET}"))),
    )]))
    .unwrap_err();
    assert_eq!(
        error.location(),
        Some(&ConfigLocation::from(&ConfigPath::parse("value").unwrap()).entry(0, MapRole::Key))
    );
    assert!(error.origin().is_none());
    assert_safe(&error, SECRET);

    let error = read::<BTreeMap<String, Numeric>>(object(vec![
        ("aaa", object(vec![("port", LayerValue::unsigned(80))])),
        (
            SECRET,
            object(vec![(
                "port",
                LayerValue::text(SECRET)
                    .with_origin(Origin::environment(format!("APP__{SECRET}__PORT"))),
            )]),
        ),
    ]))
    .unwrap_err();
    assert_eq!(
        error.location(),
        Some(
            &ConfigLocation::from(&ConfigPath::parse("value").unwrap())
                .entry(1, MapRole::Value)
                .field("port")
        )
    );
    assert!(error.origin().is_none());
    assert_safe(&error, SECRET);
}

// 在 Visitor 内消费完全部键值后失败，构造没有具体失败条目的 Map 容器级错误。
#[derive(Debug)]
struct RejectAfterMapVisitor;

impl<'de> Deserialize<'de> for RejectAfterMapVisitor {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MapVisitor;
        impl<'de> Visitor<'de> for MapVisitor {
            type Value = RejectAfterMapVisitor;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a rejected map")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                while access.next_entry::<String, bool>()?.is_some() {}
                Err(de::Error::custom("whole-map validation failed"))
            }
        }
        deserializer.deserialize_map(MapVisitor)
    }
}

// 先让普通 BTreeMap 完整反序列化，再由外层 Deserialize 失败，验证来源抑制
// 能跨过成功的内部读取继续生效。
#[derive(Debug)]
struct RejectAfterMapRead;

impl<'de> Deserialize<'de> for RejectAfterMapRead {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let _map = BTreeMap::<String, bool>::deserialize(deserializer)?;
        Err(de::Error::custom("whole-map validation failed"))
    }
}

// 再增加一层 newtype，防止包装适配器在处理错误时恢复不安全来源。
#[derive(Debug, Deserialize)]
struct RejectWrappedMap(RejectAfterMapRead);

// 动态 Map 位于普通结构体内部；外层在子对象成功后拒绝整值，用于验证嵌套传播。
#[derive(Debug)]
struct RejectAfterNestedMap;

impl<'de> Deserialize<'de> for RejectAfterNestedMap {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Envelope {
            entries: BTreeMap<String, bool>,
        }
        let envelope = Envelope::deserialize(deserializer)?;
        let _entries = envelope.entries;
        Err(de::Error::custom("nested-map validation failed"))
    }
}

// 三条完整读取后失败的路径都应保持对象级位置，但清除可能包含动态键的来源。
// 该回归专门防止“位置中没有 entry 就恢复 origin”的安全漏洞再次出现。
#[test]
fn whole_map_errors_do_not_fall_back_to_a_dynamic_environment_origin() {
    const SECRET: &str = "SECRET_DYNAMIC_KEY_SENTINEL";
    let value = || {
        object(vec![(SECRET, LayerValue::boolean(true))])
            .with_origin(Origin::environment(format!("APP__{SECRET}")))
    };
    for error in [
        read::<RejectAfterMapVisitor>(value()).unwrap_err(),
        read::<RejectAfterMapRead>(value()).unwrap_err(),
        read::<RejectWrappedMap>(value()).unwrap_err(),
    ] {
        assert_eq!(error.kind(), ConfigErrorKind::Binding);
        assert_eq!(
            error.location(),
            Some(&ConfigLocation::from(&ConfigPath::parse("value").unwrap()))
        );
        assert!(error.origin().is_none());
        assert_safe(&error, SECRET);
    }
}

// 来源抑制必须向外穿过嵌套结构体，并在 Map 形状检查失败时也成立；不能只有
// 真正进入 next_key/next_value 的路径才获得保护。
#[test]
fn nested_map_and_map_shape_errors_also_suppress_fallback_origins() {
    const SECRET: &str = "SECRET_NESTED_MAP_KEY_SENTINEL";
    let error = read::<RejectAfterNestedMap>(
        object(vec![(
            "entries",
            object(vec![(SECRET, LayerValue::boolean(true))]),
        )])
        .with_origin(Origin::environment(format!("APP__{SECRET}"))),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::Binding);
    assert!(error.origin().is_none());
    assert_safe(&error, SECRET);

    let error = read::<BTreeMap<String, bool>>(
        LayerValue::null().with_origin(Origin::environment(format!("APP__{SECRET}"))),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::TypeMismatch);
    assert!(error.origin().is_none());
    assert_safe(&error, SECRET);
}

// Map 的字符串键可按目标键类型显式转换，同时值仍独立按目标值类型处理，
// 不会因为键可解析为数值而猜测其他文本值的类型。
#[test]
fn map_keys_can_be_typed_without_inferring_value_types() {
    let value =
        read::<BTreeMap<u16, bool>>(object(vec![("42", LayerValue::text("true"))])).unwrap();
    assert_eq!(value, BTreeMap::from([(42, true)]));
    let value =
        read::<BTreeMap<char, String>>(object(vec![("配", LayerValue::text("42"))])).unwrap();
    assert_eq!(value, BTreeMap::from([('配', "42".into())]));
}

// 未知字段和未知枚举标签都是不可信输入。检查单键对象及字符串两种枚举形式，
// 确认错误不会保存名称，未知对象键对应的来源也不会恢复该名称。
#[test]
fn unknown_struct_fields_and_enum_tags_are_not_retained_in_errors() {
    const SECRET: &str = "unknown-private-field-or-variant";
    let error = read::<Database>(object(vec![
        ("host", LayerValue::string("localhost")),
        (
            SECRET,
            LayerValue::string(SECRET).with_origin(Origin::environment(format!("APP__{SECRET}"))),
        ),
    ]))
    .unwrap_err();
    assert_safe(&error, SECRET);
    assert!(error.location().unwrap().has_entries());
    assert!(error.origin().is_none());
    let error = read::<Choice>(LayerValue::string(SECRET)).unwrap_err();
    assert_safe(&error, SECRET);
    let error = read::<Choice>(object(vec![(
        SECRET,
        LayerValue::null().with_origin(Origin::environment(format!("APP__{SECRET}"))),
    )]))
    .unwrap_err();
    assert_safe(&error, SECRET);
    assert!(error.origin().is_none());
}

// 业务 Deserialize 主动构造含敏感文本的错误，用来验证消息不会透明传递到公共错误。
#[derive(Debug)]
struct RejectWithPrivateMessage;

impl<'de> Deserialize<'de> for RejectWithPrivateMessage {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        Err(de::Error::custom("password=private-user-error-sentinel"))
    }
}

// 更强的对照：错误参数一旦被格式化就 panic，证明实现没有先格式化再尝试打码。
#[derive(Debug)]
struct RejectWithoutFormatting;

impl<'de> Deserialize<'de> for RejectWithoutFormatting {
    fn deserialize<D: Deserializer<'de>>(_deserializer: D) -> Result<Self, D::Error> {
        struct DangerousDisplay;
        impl fmt::Display for DangerousDisplay {
            fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
                panic!("custom deserialization error messages must never be formatted")
            }
        }
        Err(de::Error::custom(DangerousDisplay))
    }
}

// 自定义消息和失败输入均不能保留，source 也不得偷偷挂接原始业务错误；
// DangerousDisplay 的用例同时验证不执行用户错误参数的格式化逻辑。
#[test]
fn custom_deserialization_errors_neither_format_nor_retain_private_messages() {
    let error = read::<RejectWithPrivateMessage>(LayerValue::string("input-secret")).unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::Binding);
    assert_safe(&error, "private-user-error-sentinel");
    assert_safe(&error, "input-secret");
    assert!(error.source().is_none());
    let error = read::<RejectWithoutFormatting>(LayerValue::null()).unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::Binding);
}

// 手写字节 Visitor，覆盖字符串的字节通道和数组的逐项通道，不增加 serde_bytes 依赖。
#[derive(Debug, PartialEq)]
struct Bytes(Vec<u8>);

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Bytes;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("bytes")
            }
            fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<Bytes, E> {
                Ok(Bytes(bytes.to_vec()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut access: A) -> Result<Bytes, A::Error> {
                let mut bytes = Vec::new();
                while let Some(byte) = access.next_element()? {
                    bytes.push(byte);
                }
                Ok(Bytes(bytes))
            }
        }
        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

// 字符串按 UTF-8 原字节交付，数值数组按 u8 范围检查，不能截断 256 等越界值。
#[test]
fn byte_visitors_accept_utf8_and_typed_byte_sequences() {
    assert_eq!(
        read::<Bytes>(LayerValue::string("配置")).unwrap(),
        Bytes("配置".as_bytes().to_vec())
    );
    assert_eq!(
        read::<Bytes>(LayerValue::array(vec![
            LayerValue::unsigned(0),
            LayerValue::unsigned(255)
        ]))
        .unwrap(),
        Bytes(vec![0, 255])
    );
    assert!(read::<Bytes>(LayerValue::array(vec![LayerValue::unsigned(256)])).is_err());
}
