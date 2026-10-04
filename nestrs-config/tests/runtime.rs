//! 独立配置运行时的公开接口验收。
//!
//! 这些测试只通过用户可见的 API 构造来源、读取配置，覆盖合并、路径、来源、快照
//! 所有权、资源预算和诊断安全。末尾的独立 Cargo 项目还验证依赖重命名，以及运行时
//! 确实不需要 Nestrs core、编译器桥接或 Tokio，避免仅在当前 workspace 中偶然通过。

use nestrs_config::sources::Memory;
use nestrs_config::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigOrigins, ConfigPath, ConfigService,
    ConfigSource, Configuration, HistoryStatus, LayerValue, LoadContext, Origin,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::error::Error;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

// 构造具名测试层：每个条目都走公开的路径解析、插入和预算检查。
fn layer(name: &str, entries: Vec<(&str, LayerValue)>) -> ConfigLayer {
    let mut layer = ConfigLayer::builder(Origin::named(name));
    for (path, value) in entries {
        layer
            .insert(ConfigPath::parse(path).unwrap(), value)
            .unwrap();
    }
    layer.build().unwrap()
}

// 为不需要多层覆盖的用例创建一份最小快照。
fn configuration(entries: Vec<(&str, LayerValue)>) -> Configuration {
    Configuration::builder()
        .add_source(Memory::new(layer("test", entries)))
        .build()
        .unwrap()
}

// 保留公开 object 构造器的重复键校验，不直接绕过内部值模型。
fn object(entries: Vec<(&str, LayerValue)>) -> LayerValue {
    LayerValue::object(entries.into_iter().map(|(k, v)| (k.into(), v)).collect()).unwrap()
}

// 核对逐键对象合并、数组整体替换及后添加来源优先，避免把数组误做逐项拼接。
#[test]
fn layers_merge_objects_replace_arrays_and_override_scalars_in_order() {
    let config = Configuration::builder()
        .add_source(Memory::new(layer(
            "defaults",
            vec![
                ("db.host", LayerValue::string("localhost")),
                ("db.port", LayerValue::integer(5432)),
                ("db.pool", LayerValue::integer(16)),
                (
                    "replicas",
                    LayerValue::array(vec![object(vec![
                        ("host", LayerValue::string("old")),
                        ("port", LayerValue::integer(100)),
                    ])]),
                ),
            ],
        )))
        .add_source(Memory::new(layer(
            "override",
            vec![
                ("db.port", LayerValue::text("6432")),
                (
                    "replicas",
                    LayerValue::array(vec![object(vec![("host", LayerValue::string("new"))])]),
                ),
            ],
        )))
        .add_source(Memory::new(layer("empty", vec![("db", object(vec![]))])))
        .build()
        .unwrap();
    assert_eq!(
        config.get_required::<String>("db.host").unwrap(),
        "localhost"
    );
    assert_eq!(config.get_required::<u16>("db.port").unwrap(), 6432);
    assert_eq!(config.get_required::<u32>("db.pool").unwrap(), 16);
    let replicas: Vec<BTreeMap<String, String>> = config.get_required("replicas").unwrap();
    assert_eq!(
        replicas,
        vec![BTreeMap::from([("host".into(), "new".into())])]
    );
}

// 缺失、null、空字符串和空对象必须保持不同语义；Option 的内外两层含义不能混淆。
#[test]
fn null_missing_and_empty_values_are_distinct() {
    let config = Configuration::builder()
        .add_source(Memory::new(layer(
            "base",
            vec![
                ("value", LayerValue::integer(1)),
                ("replace_null", LayerValue::null()),
                ("items", LayerValue::array(vec![LayerValue::integer(1)])),
            ],
        )))
        .add_source(Memory::new(layer(
            "next",
            vec![
                ("value", LayerValue::null()),
                ("replace_null", LayerValue::string("")),
                ("items", LayerValue::array(vec![])),
                ("empty", object(vec![])),
            ],
        )))
        .build()
        .unwrap();
    assert_eq!(config.get::<Option<u16>>("missing").unwrap(), None);
    assert_eq!(config.get::<Option<u16>>("value").unwrap(), Some(None));
    assert_eq!(config.get_required::<Option<u16>>("value").unwrap(), None);
    assert_eq!(config.get_required::<String>("replace_null").unwrap(), "");
    assert!(config.get_required::<Vec<u16>>("items").unwrap().is_empty());
    assert!(
        config
            .get_required::<BTreeMap<String, u16>>("empty")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        config.get_required::<u16>("missing").unwrap_err().kind(),
        ConfigErrorKind::Missing
    );
    assert!(config.get::<u16>("value").is_err());
    assert!(config.get::<u16>("value.child").is_err());
}

// 非 null 的对象/数组/标量发生形态冲突时拒绝合并；null 则允许双向整体替换。
#[test]
fn shape_conflicts_are_errors_but_null_can_replace_every_shape() {
    let shapes = [
        LayerValue::integer(1),
        object(vec![]),
        LayerValue::array(vec![]),
    ];
    for (left_i, left) in shapes.iter().enumerate() {
        for (right_i, right) in shapes.iter().enumerate() {
            let result = Configuration::builder()
                .add_source(Memory::new(layer("base", vec![("value", left.clone())])))
                .add_source(Memory::new(layer("next", vec![("value", right.clone())])))
                .build();
            if left_i == right_i {
                assert!(result.is_ok());
            } else {
                assert_eq!(result.unwrap_err().kind(), ConfigErrorKind::MergeConflict);
            }
        }
        for (first, second) in [
            (left.clone(), LayerValue::null()),
            (LayerValue::null(), left.clone()),
        ] {
            assert!(
                Configuration::builder()
                    .add_source(Memory::new(layer("base", vec![("value", first)])))
                    .add_source(Memory::new(layer("next", vec![("value", second)])))
                    .build()
                    .is_ok()
            );
        }
    }
}

// 同层路径不能重复占用，非法数组插入路径也不能绕过来源构造检查。
#[test]
fn one_layer_rejects_duplicate_and_parent_child_paths() {
    for (first, second) in [("a", "a"), ("a", "a.b"), ("a.b", "a")] {
        let mut builder = ConfigLayer::builder(Origin::named("test"));
        builder
            .insert(ConfigPath::parse(first).unwrap(), LayerValue::integer(1))
            .unwrap();
        assert!(
            builder
                .insert(ConfigPath::parse(second).unwrap(), LayerValue::integer(2))
                .is_err()
        );
    }
    assert!(
        LayerValue::object(vec![
            ("key".into(), LayerValue::integer(1)),
            ("key".into(), LayerValue::integer(2))
        ])
        .is_err()
    );
    let mut builder = ConfigLayer::builder(Origin::named("test"));
    assert!(
        builder
            .insert(
                ConfigPath::root().key("items").index(0),
                LayerValue::integer(1)
            )
            .is_err()
    );
    assert!(LayerValue::float(f64::NAN).is_err());
    assert!(LayerValue::float(f64::INFINITY).is_err());
}

// 显式插入的空对象也已经占用路径；这与为兄弟字段自动创建的隐式父对象不同。
#[test]
fn explicit_parent_insertions_cannot_be_reopened_with_child_paths() {
    for (first, second) in [("a", "a.b"), ("a.b", "a")] {
        let mut builder = ConfigLayer::builder(Origin::named("test"));
        builder
            .insert(ConfigPath::parse(first).unwrap(), object(vec![]))
            .unwrap();
        assert!(
            builder
                .insert(ConfigPath::parse(second).unwrap(), object(vec![]))
                .is_err()
        );
    }
}

// 用带点、括号及空字符串的键验证结构化地址；数字键与数组索引必须明确区分。
#[test]
fn explicit_paths_preserve_literal_keys_and_array_indices() {
    let config = configuration(vec![
        (
            "special",
            object(vec![(
                "a.b",
                object(vec![("[x]", object(vec![("", LayerValue::integer(7))]))]),
            )]),
        ),
        (
            "items",
            LayerValue::array(vec![LayerValue::integer(10), LayerValue::integer(20)]),
        ),
        ("numeric", object(vec![("0", LayerValue::integer(30))])),
    ]);
    let special = ConfigPath::root()
        .key("special")
        .key("a.b")
        .key("[x]")
        .key("");
    assert_eq!(config.get_required_path::<u16>(&special).unwrap(), 7);
    assert_eq!(
        config
            .get_required_path::<u16>(&ConfigPath::root().key("items").index(1))
            .unwrap(),
        20
    );
    assert_eq!(
        config
            .get_path::<u16>(&ConfigPath::root().key("items").index(99))
            .unwrap(),
        None
    );
    assert_eq!(config.get_required::<u16>("numeric.0").unwrap(), 30);
    assert!(config.get::<u16>("items.0").is_err());
    assert!(
        config
            .get_path::<u16>(&ConfigPath::root().key("numeric").index(0))
            .is_err()
    );
    for path in [".a", "a.", "a..b", "items[0]", "x]", "["] {
        assert_eq!(
            config.get::<u16>(path).unwrap_err().kind(),
            ConfigErrorKind::InvalidPath
        );
    }
    assert!(config.section("").is_ok());
}

#[derive(Debug, Deserialize, PartialEq)]
// 普通 Serde 数据模型，用于证明 section 绑定不需要 Nestrs 声明宏。
struct Database {
    host: String,
    port: u16,
}

// 验证视图的拥有所有权、标量/null 绑定，以及固有/泛型读取都保留完整父路径。
#[test]
fn section_owns_snapshot_binds_scalars_and_uses_relative_paths() {
    let config = configuration(vec![
        ("app.db.host", LayerValue::string("localhost")),
        ("app.db.port", LayerValue::integer(5432)),
        ("nothing", LayerValue::null()),
    ]);
    let app = config.section("app").unwrap();
    let db = app.section("db").unwrap();
    assert_eq!(
        db.bind::<Database>().unwrap(),
        Database {
            host: "localhost".into(),
            port: 5432
        }
    );
    assert_eq!(app.get_required::<u16>("db.port").unwrap(), 5432);
    assert_eq!(db.section("port").unwrap().bind::<u16>().unwrap(), 5432);
    assert_eq!(
        config
            .section("nothing")
            .unwrap()
            .bind::<Option<u16>>()
            .unwrap(),
        None
    );
    assert_eq!(
        config.section("missing").unwrap_err().kind(),
        ConfigErrorKind::Missing
    );
    assert_eq!(
        db.get_required::<u16>("missing").unwrap_err().location(),
        config
            .get_required::<u16>("app.db.missing")
            .unwrap_err()
            .location()
    );
    fn required_from_service<C: ConfigService>(config: &C) -> Result<u16, ConfigError> {
        config.get_required("missing")
    }
    assert_eq!(
        required_from_service(&db).unwrap_err().location(),
        config
            .get_required::<u16>("app.db.missing")
            .unwrap_err()
            .location(),
    );
    assert_eq!(
        db.explain("port").unwrap().path(),
        &ConfigPath::parse("app.db.port").unwrap()
    );
    drop(config);
    assert_eq!(db.get_required::<u16>("port").unwrap(), 5432);
}

// 来源解释只表示当前已知事实；未知来源和没有记录覆盖历史不能被伪装成单一来源。
#[test]
fn explain_distinguishes_single_merged_and_unknown_origins_without_history() {
    let config = Configuration::builder()
        .add_source(Memory::new(layer(
            "base",
            vec![
                ("db.host", LayerValue::string("localhost")),
                ("db.port", LayerValue::integer(5432)),
            ],
        )))
        .add_source(Memory::new(layer(
            "override",
            vec![("db.port", LayerValue::integer(6432))],
        )))
        .build()
        .unwrap();
    assert_eq!(
        config.explain("db.host").unwrap().origins(),
        &ConfigOrigins::Single(Origin::named("base"))
    );
    assert_eq!(
        config.explain("db.port").unwrap().origins(),
        &ConfigOrigins::Single(Origin::named("override"))
    );
    match config.explain("db").unwrap().origins() {
        ConfigOrigins::Merged(origins) => {
            assert!(origins.contains(&Origin::named("base")));
            assert!(origins.contains(&Origin::named("override")));
        }
        other => panic!("expected merged origins, got {other:?}"),
    }
    assert_eq!(
        config.explain("db").unwrap().history(),
        HistoryStatus::Unavailable
    );
    let empty = Configuration::builder().build().unwrap();
    assert_eq!(
        empty.explain("").unwrap().origins(),
        &ConfigOrigins::Unknown
    );
    assert!(config.explain("missing").is_err());
}

// 完全覆盖后旧来源必须消失，节点单独指定的来源仍应保留，不能用层名称统一覆盖。
#[test]
fn explain_discards_fully_overridden_origins_and_preserves_node_origins() {
    let replacement = Origin::named("replacement");
    let config = Configuration::builder()
        .add_source(Memory::new(layer(
            "obsolete",
            vec![
                ("db.host", LayerValue::string("old")),
                ("db.port", LayerValue::integer(10)),
            ],
        )))
        .add_source(Memory::new(layer(
            "replacement",
            vec![
                ("db.host", LayerValue::string("new")),
                ("db.port", LayerValue::integer(20)),
            ],
        )))
        .build()
        .unwrap();
    assert_eq!(
        config.explain("db").unwrap().origins(),
        &ConfigOrigins::Single(replacement)
    );
    let config = Configuration::builder()
        .add_source(Memory::new(layer(
            "container",
            vec![(
                "port",
                LayerValue::integer(10).with_origin(Origin::named("custom-node")),
            )],
        )))
        .build()
        .unwrap();
    assert_eq!(
        config.explain("port").unwrap().origins(),
        &ConfigOrigins::Single(Origin::named("custom-node"))
    );
}

// 空对象没有子节点可供汇总，但仍可能具有明确来源；防止合并时误清为空未知来源。
#[test]
fn explicit_empty_objects_keep_known_origins_through_merge() {
    let empty = Configuration::builder()
        .add_source(Memory::new(layer("known-empty", vec![])))
        .build()
        .unwrap();
    assert_eq!(
        empty.explain("").unwrap().origins(),
        &ConfigOrigins::Single(Origin::named("known-empty"))
    );
    let nested = Configuration::builder()
        .add_source(Memory::new(layer("base", vec![("empty", object(vec![]))])))
        .add_source(Memory::new(layer(
            "override",
            vec![("empty", object(vec![]))],
        )))
        .build()
        .unwrap();
    assert!(!matches!(
        nested.explain("empty").unwrap().origins(),
        ConfigOrigins::Unknown
    ));
}

// 通过计数器观测加载次数；可变输入在 build 后改变，用来区分快照与实时读取。
struct CountingSource {
    calls: Arc<AtomicUsize>,
    value: Arc<AtomicUsize>,
}
impl ConfigSource for CountingSource {
    fn load(&self, _: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(layer(
            "custom",
            vec![(
                "value",
                LayerValue::unsigned(self.value.load(Ordering::SeqCst) as u64),
            )],
        ))
    }
}

// 先检查 add_source 不加载，再改变来源输入并跨线程读取，证明 clone/section 共享固定快照。
#[test]
fn build_loads_once_and_all_readers_share_an_immutable_snapshot() {
    fn thread_safe<T: Send + Sync>() {}
    thread_safe::<Configuration>();
    let calls = Arc::new(AtomicUsize::new(0));
    let value = Arc::new(AtomicUsize::new(4));
    let builder = Configuration::builder().add_source(CountingSource {
        calls: calls.clone(),
        value: value.clone(),
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let config = builder.build().unwrap();
    let clone = config.clone();
    let section = config.section("value").unwrap();
    value.store(99, Ordering::SeqCst);
    let handle = std::thread::spawn(move || clone.get_required::<u64>("value").unwrap());
    assert_eq!(handle.join().unwrap(), 4);
    assert_eq!(config.get_required::<u64>("value").unwrap(), 4);
    assert_eq!(section.bind::<u64>().unwrap(), 4);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

// 记录来源调用顺序，并允许中途失败，验证首错返回不会继续加载后续来源。
struct OrderedSource {
    order: Arc<Mutex<Vec<usize>>>,
    index: usize,
    fail: bool,
}
impl ConfigSource for OrderedSource {
    fn load(&self, _: &LoadContext) -> Result<ConfigLayer, ConfigError> {
        self.order.lock().unwrap().push(self.index);
        if self.fail {
            return Err(ConfigError::missing("source"));
        }
        Ok(layer(
            "ordered",
            vec![("value", LayerValue::unsigned(self.index as u64))],
        ))
    }
}

// 前一来源失败时立即停止，既不继续执行后续 load，也不交付部分构建结果。
#[test]
fn sources_stop_at_first_error_without_loading_later_sources() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let mut builder = Configuration::builder();
    for index in 0..3 {
        builder = builder.add_source(OrderedSource {
            order: order.clone(),
            index,
            fail: index == 1,
        });
    }
    assert!(builder.build().is_err());
    assert_eq!(*order.lock().unwrap(), vec![0, 1]);
}

// 泛型调用方只需要 ConfigService 的两个方法，不能暗中依赖官方节点或 builder。
#[test]
fn generic_protocol_requires_only_typed_getters() {
    fn read<C: ConfigService>(config: &C) -> Result<u16, ConfigError> {
        assert_eq!(config.get::<String>("missing")?, None);
        config.get_required("port")
    }
    assert_eq!(
        read(&configuration(vec![("port", LayerValue::integer(80))])).unwrap(),
        80
    );
}

// 第三方客户端直接提供自己的 Serde 解码器，并复用默认 get_required，证明协议边界独立。
#[test]
fn third_party_client_can_implement_only_the_generic_get_method() {
    struct ThirdParty;
    impl ConfigService for ThirdParty {
        fn get<T: serde::de::DeserializeOwned>(
            &self,
            path: &str,
        ) -> Result<Option<T>, ConfigError> {
            if path != "port" {
                return Ok(None);
            }
            let value = serde::de::value::U16Deserializer::<serde::de::value::Error>::new(8080);
            T::deserialize(value)
                .map(Some)
                .map_err(|_| ConfigError::new(ConfigErrorKind::Binding))
        }
    }
    assert_eq!(ThirdParty.get_required::<u16>("port").unwrap(), 8080);
    assert_eq!(
        ThirdParty
            .get_required::<u16>("missing")
            .unwrap_err()
            .kind(),
        ConfigErrorKind::Missing
    );
}

// 用秘密哨兵和会 panic 的自定义 Debug 检查整条默认诊断路径；显式读取仍必须返回真实值。
#[test]
fn default_and_pretty_debug_do_not_format_values_or_custom_sources() {
    const SECRET: &str = "CONFIG_PRIVATE_SENTINEL_798248";
    // 如果 builder 试图格式化内部来源，本用例会立即失败，而不是只做字符串匹配。
    struct HostileDebug;
    impl std::fmt::Debug for HostileDebug {
        fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            panic!("must not format sources")
        }
    }
    impl ConfigSource for HostileDebug {
        fn load(&self, _: &LoadContext) -> Result<ConfigLayer, ConfigError> {
            Ok(layer("safe", vec![("password", LayerValue::text(SECRET))]))
        }
    }
    fn safe(value: &impl std::fmt::Debug) {
        assert!(!format!("{value:?}").contains(SECRET));
        assert!(!format!("{value:#?}").contains(SECRET));
    }
    let value = object(vec![(SECRET, LayerValue::string(SECRET))]);
    safe(&value);
    let source_layer = layer(
        "safe",
        vec![("password", LayerValue::text(SECRET)), ("map", value)],
    );
    safe(&source_layer);
    let source = Memory::new(source_layer);
    safe(&source);
    let builder = Configuration::builder()
        .add_source(source)
        .add_source(HostileDebug);
    safe(&builder);
    let config = builder.build().unwrap();
    safe(&config);
    safe(&config.section("password").unwrap());
    safe(&config.explain("password").unwrap());
    let error = config.get_required::<u16>("password").unwrap_err();
    safe(&error);
    assert!(!error.to_string().contains(SECRET));
    assert!(error.source().is_none());
    assert_eq!(config.get_required::<String>("password").unwrap(), SECRET);
}

// 超深和过宽的内存输入都必须返回受控 Limit 错误，不能只给文件来源设置预算。
#[test]
fn layer_depth_and_node_budgets_fail_with_structured_errors() {
    let mut value = LayerValue::integer(0);
    for _ in 0..70 {
        value = LayerValue::array(vec![value]);
    }
    let mut builder = ConfigLayer::builder(Origin::named("budget"));
    let result = builder
        .insert(ConfigPath::parse("deep").unwrap(), value)
        .and_then(|()| builder.build());
    assert_eq!(result.unwrap_err().kind(), ConfigErrorKind::Limit);
    let mut builder = ConfigLayer::builder(Origin::named("budget"));
    let result = builder
        .insert(
            ConfigPath::parse("wide").unwrap(),
            LayerValue::array((0..100_001).map(|_| LayerValue::null()).collect()),
        )
        .and_then(|()| builder.build());
    assert_eq!(result.unwrap_err().kind(), ConfigErrorKind::Limit);
}

// 预算边界本身应合法；插入路径与子树各自合法但组合超限时仍必须拒绝。
#[test]
fn resource_budgets_accept_the_documented_boundary_and_count_full_paths() {
    let mut builder = ConfigLayer::builder(Origin::named("boundary"));
    let mut path = ConfigPath::root();
    for _ in 0..nestrs_config::MAX_DEPTH {
        path = path.key("next");
    }
    builder
        .insert(path.clone(), LayerValue::integer(9))
        .unwrap();
    let config = Configuration::builder()
        .add_source(Memory::new(builder.build().unwrap()))
        .build()
        .unwrap();
    assert_eq!(config.get_required_path::<u16>(&path).unwrap(), 9);

    let mut builder = ConfigLayer::builder(Origin::named("boundary"));
    builder
        .insert(
            ConfigPath::root().key("items"),
            LayerValue::array(
                (0..nestrs_config::MAX_NODES - 2)
                    .map(|_| LayerValue::null())
                    .collect(),
            ),
        )
        .unwrap();
    let config = Configuration::builder()
        .add_source(Memory::new(builder.build().unwrap()))
        .build()
        .unwrap();
    assert_eq!(
        config
            .get_required::<Vec<Option<u8>>>("items")
            .unwrap()
            .len(),
        nestrs_config::MAX_NODES - 2
    );

    let mut builder = ConfigLayer::builder(Origin::named("combined-depth"));
    let mut value = LayerValue::integer(1);
    for _ in 0..5 {
        value = LayerValue::array(vec![value]);
    }
    // 路径长度和子树深度分别合法，组合后的完整树却超过限制。
    assert!(
        builder
            .insert(path, value)
            .and_then(|()| builder.build())
            .is_err()
    );
}

// 各层分别未超限，不代表最终合并树合法；结果节点总数也必须重新检查。
#[test]
fn merged_tree_budget_is_enforced_even_when_each_source_fits() {
    let config = Configuration::builder()
        .add_source(Memory::new(layer(
            "left",
            vec![(
                "left",
                LayerValue::array((0..60_000).map(|_| LayerValue::null()).collect()),
            )],
        )))
        .add_source(Memory::new(layer(
            "right",
            vec![(
                "right",
                LayerValue::array((0..60_000).map(|_| LayerValue::null()).collect()),
            )],
        )))
        .build();
    assert_eq!(config.unwrap_err().kind(), ConfigErrorKind::Limit);
}

// 启动真实独立 Cargo 项目，验证运行结果与依赖树；避免仅由同 workspace 的依赖合并掩盖问题。
#[test]
fn standalone_cargo_project_uses_renamed_dependency_without_nestrs_or_tokio() {
    use std::process::Command;
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/standalone/Cargo.toml");
    // 子 Cargo 使用独立 target，避免与正在执行本测试的外层 Cargo 争用构建锁。
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../target/nestrs-config-standalone-acceptance");
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(&cargo)
        .args(["run", "--quiet", "--locked", "--offline", "--manifest-path"])
        .arg(&fixture)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("start standalone Cargo build");
    assert!(
        output.status.success(),
        "standalone failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("standalone config acceptance passed")
    );
    // 编译成功还不够：读取真实依赖树，验证没有经传递依赖重新引入框架或异步运行时。
    let tree = Command::new(cargo)
        .args([
            "tree",
            "--locked",
            "--offline",
            "--prefix",
            "none",
            "--manifest-path",
        ])
        .arg(&fixture)
        .env("CARGO_TARGET_DIR", target)
        .output()
        .expect("inspect standalone dependency tree");
    assert!(
        tree.status.success(),
        "{}",
        String::from_utf8_lossy(&tree.stderr)
    );
    let tree = String::from_utf8(tree.stdout).unwrap();
    for line in tree.lines() {
        let name = line.split_whitespace().next().unwrap_or_default();
        assert!(
            !matches!(
                name,
                "nestrs-core" | "cargo-nestrs" | "nestrs-tool-bridge" | "tokio"
            ),
            "unexpected runtime dependency: {line}"
        );
    }
    assert!(tree.lines().any(|line| line.starts_with("nestrs-config ")));
}
