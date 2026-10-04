//! 配置来源的行为与安全边界回归。
//!
//! 内存和显式环境输入覆盖默认可用功能；真实进程环境通过隔离子进程验证，避免修改
//! 并行测试共享的环境。文件用例按 feature 启用，检查解析、来源定位、覆盖、资源限制
//! 和错误脱敏；可能阻塞的 FIFO 路径也只在有超时保护的子进程中执行。

use nestrs_config::{
    ConfigError, ConfigErrorKind, ConfigLayer, ConfigPath, Configuration, LayerValue, Origin,
    sources::{Environment, Memory},
};

// 同时检查 Display、Debug 和完整 cause 链，防止仅遮罩最外层错误而泄露原值。
fn assert_safe(error: &ConfigError, secret: &str) {
    assert!(!format!("{error}").contains(secret));
    assert!(!format!("{error:?}").contains(secret));
    let mut cause = std::error::Error::source(error);
    while let Some(error) = cause {
        assert!(!format!("{error}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
        cause = error.source();
    }
}

#[test]
// 验证来源释放后快照仍可用，并区分可按目标转数值的 Text 与保持原类型的 String。
fn memory_is_owned_and_preserves_text_vs_string() {
    let mut layer = ConfigLayer::builder(Origin::named("test"));
    layer
        .insert(ConfigPath::root().key("text"), LayerValue::text("42"))
        .unwrap();
    layer
        .insert(ConfigPath::root().key("string"), LayerValue::string("42"))
        .unwrap();
    let source = Memory::new(layer.build().unwrap());
    let config = Configuration::builder()
        .add_source(source.clone())
        .build()
        .unwrap();
    drop(source);
    assert_eq!(config.get_required::<u16>("text").unwrap(), 42);
    assert_eq!(config.get_required::<String>("string").unwrap(), "42");
    assert!(config.get_required::<u16>("string").is_err());
}

#[test]
// 验证精确前缀筛选、ASCII 小写及单下划线保留，并确认纯数字路径段仍是对象键。
fn environment_maps_only_matching_prefix_and_ascii_case() {
    let config = Configuration::builder()
        .add_source(Environment::from_iter(
            "APP__",
            [
                ("APP__HTTP__PORT", "3000"),
                ("APP__DB__POOL_SIZE", "16"),
                ("APP__ITEMS__0", "first"),
                ("OTHER__HTTP__PORT", "invalid"),
                ("app__HTTP__PORT", "also-invalid"),
            ],
        ))
        .build()
        .unwrap();
    assert_eq!(config.get_required::<u16>("http.port").unwrap(), 3000);
    assert_eq!(config.get_required::<u32>("db.pool_size").unwrap(), 16);
    assert_eq!(config.get_required::<String>("items.0").unwrap(), "first");
    assert_eq!(config.get::<String>("HTTP.PORT").unwrap(), None);
    assert!(
        config
            .get_path::<String>(&ConfigPath::root().key("items").index(0))
            .is_err()
    );
}

// 专用前缀与子入口标记只通过 Command 的子进程环境传递，不写父进程全局环境。
const PROCESS_ENV_PREFIX: &str = "NESTRS_CONFIG_SOURCE_PROCESS_7F14__";
const PROCESS_ENV_CHILD: &str = "NESTRS_CONFIG_SOURCE_PROCESS_7F14_CHILD";

#[test]
// 通过 Command 只设置子进程环境，验证真实环境读取；比较父进程前后快照并限制等待时间。
fn process_environment_is_read_in_isolated_child_without_parent_mutation() {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    let inputs = [
        (format!("{PROCESS_ENV_PREFIX}APP__PORT"), "4317"),
        (format!("{PROCESS_ENV_PREFIX}ENABLED"), "true"),
        (format!("{PROCESS_ENV_PREFIX}TEXT"), "0017"),
        (
            "NESTRS_CONFIG_SOURCE_PROCESS_7F14_OTHER__IGNORED".into(),
            "outside prefix",
        ),
        (
            format!("{}WRONGCASE", PROCESS_ENV_PREFIX.to_ascii_lowercase()),
            "different prefix spelling",
        ),
        (PROCESS_ENV_CHILD.into(), "1"),
    ];
    let snapshot = || {
        inputs
            .iter()
            .map(|(key, _)| (key.clone(), std::env::var_os(key)))
            .collect::<Vec<_>>()
    };
    let before = snapshot();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "environment_process_child", "--nocapture"])
        .envs(inputs.iter().map(|(key, value)| (key, value)))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let output = loop {
        if child.try_wait().unwrap().is_some() {
            break child.wait_with_output().unwrap();
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("process environment source child did not finish");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(snapshot(), before, "the parent environment changed");
    assert!(
        output.status.success(),
        "environment child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
// 隔离环境用例的专用入口：普通测试运行时直接返回，子进程检查类型读取、筛选及来源名称。
fn environment_process_child() {
    if std::env::var_os(PROCESS_ENV_CHILD).is_none() {
        return;
    }
    let config = Configuration::builder()
        .add_source(Environment::with_prefix(PROCESS_ENV_PREFIX).separator("__"))
        .build()
        .unwrap();
    assert_eq!(config.get_required::<u16>("app.port").unwrap(), 4317);
    assert!(config.get_required::<bool>("enabled").unwrap());
    assert_eq!(config.get_required::<String>("text").unwrap(), "0017");
    assert_eq!(config.get::<String>("ignored").unwrap(), None);
    assert_eq!(config.get::<String>("wrongcase").unwrap(), None);
    let explanation = config.explain("app.port").unwrap();
    assert_eq!(
        explanation.origins().single().unwrap().name(),
        format!("{PROCESS_ENV_PREFIX}APP__PORT")
    );
}

#[test]
// 验证环境值不提前猜类型：null 仍是文本，空字符串不是缺失，JSON/逗号文本不会变成数组。
fn environment_keeps_null_and_collections_as_plain_text() {
    let config = Configuration::builder()
        .add_source(Environment::from_iter(
            "A__",
            [
                ("A__NULL", "null"),
                ("A__ARRAY", "[1,2]"),
                ("A__CSV", "a,b"),
                ("A__EMPTY", ""),
            ],
        ))
        .build()
        .unwrap();
    assert_eq!(
        config.get_required::<Option<String>>("null").unwrap(),
        Some("null".into())
    );
    assert_eq!(config.get_required::<String>("empty").unwrap(), "");
    assert!(config.get_required::<Vec<u8>>("array").is_err());
    assert!(config.get_required::<Vec<String>>("csv").is_err());
}

#[test]
// 验证大小写规范化撞名、原始重复键及父子路径冲突都失败，并保留环境来源。
fn environment_rejects_collisions_and_parent_child_conflicts() {
    for pairs in [
        vec![("A__KEY", "one"), ("A__key", "two")],
        vec![("A__KEY", "one"), ("A__KEY", "two")],
        vec![("A__KEY", "one"), ("A__KEY__CHILD", "two")],
    ] {
        let error = Configuration::builder()
            .add_source(Environment::from_iter("A__", pairs))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::DuplicateKey);
        assert!(error.origin().is_some());
    }
}

#[test]
// 验证空路径段与空分隔符明确报错，不通过折叠空段或隐式默认值修复非法声明。
fn environment_rejects_empty_segments_and_separator() {
    for key in ["A__", "A____KEY", "A__KEY__", "A__KEY____CHILD"] {
        let error = Configuration::builder()
            .add_source(Environment::from_iter("A__", [(key, "value")]))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::InvalidOptions);
    }
    let error = Configuration::builder()
        .add_source(Environment::from_iter("A__", [("A__KEY", "value")]).separator(""))
        .build()
        .unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::InvalidOptions);
}

#[test]
// 验证环境键在构造过深路径时即返回资源限制，而不是无限展开隐式父对象。
fn environment_rejects_paths_beyond_depth_budget() {
    let key = format!("A__{}", vec!["X"; nestrs_config::MAX_DEPTH + 1].join("__"));
    let error = Configuration::builder()
        .add_source(Environment::from_iter("A__", [(key, "value")]))
        .build()
        .unwrap_err();
    assert_eq!(error.kind(), ConfigErrorKind::Limit);
}

#[test]
// 验证来源、层与快照的 Debug 不显示原值，错误链也保持安全；显式读取仍返回真实值。
fn source_debug_omits_values() {
    let secret = "secret-source-value-92f0";
    let environment = Environment::from_iter("A__", [("A__TOKEN", secret)]);
    assert!(!format!("{environment:?}").contains(secret));
    let mut layer = ConfigLayer::builder(Origin::named("test"));
    layer
        .insert(ConfigPath::root().key("token"), LayerValue::string(secret))
        .unwrap();
    let layer = layer.build().unwrap();
    assert!(!format!("{layer:?}").contains(secret));
    let memory = Memory::new(layer);
    assert!(!format!("{memory:?}").contains(secret));
    let config = Configuration::builder().add_source(memory).build().unwrap();
    assert!(!format!("{config:?}").contains(secret));
    assert_eq!(config.get_required::<String>("token").unwrap(), secret);
    assert_safe(&config.get_required::<u64>("token").unwrap_err(), secret);
}

#[cfg(unix)]
#[test]
// 在 Unix 构造真实非 Unicode 输入：无关前缀被忽略，已选中的非法键或值必须报编码错误。
fn environment_checks_non_unicode_only_inside_selected_prefix() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let config = Configuration::builder()
        .add_source(Environment::from_iter(
            "A__",
            [
                (
                    OsString::from_vec(b"OTHER_\xff".to_vec()),
                    OsString::from("x"),
                ),
                (OsString::from("A__OK"), OsString::from("yes")),
            ],
        ))
        .build()
        .unwrap();
    assert_eq!(config.get_required::<String>("ok").unwrap(), "yes");
    for pair in [
        (OsString::from_vec(b"A__\xff".to_vec()), OsString::from("x")),
        (OsString::from("A__VALUE"), OsString::from_vec(vec![0xff])),
    ] {
        let error = Configuration::builder()
            .add_source(Environment::from_iter("A__", [pair]))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Encoding);
    }
}

#[cfg(any(feature = "json", feature = "toml", feature = "dotenv"))]
mod files {
    //! 通过真实临时文件验证可选来源，所有用例独立持有目录，避免修改工作目录。

    use super::*;
    #[cfg(feature = "json")]
    use std::path::Path;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    // 每个用例创建唯一目录并在 Drop 时清理，文件删除不会影响已经构建的快照。
    struct TestDir(PathBuf);
    impl TestDir {
        // 结合进程、时间和进程内序号隔离并行用例，不复用用户已有目录。
        fn new() -> Self {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "nestrs-config-sources-{}-{now}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        // 允许写入任意字节，以覆盖非法 UTF-8，而不是只测试合法文本。
        fn write(&self, name: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }
        #[cfg(feature = "json")]
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(feature = "json")]
    // 通过临时文件与正式 Json 来源构建快照，不以直接调用内部解析器代替文件链路。
    fn json(text: &str) -> Result<Configuration, ConfigError> {
        let dir = TestDir::new();
        let path = dir.write("input.json", text);
        Configuration::builder()
            .add_source(nestrs_config::sources::Json::file(path))
            .build()
    }

    #[cfg(feature = "toml")]
    // 通过临时文件与正式 Toml 来源执行解析、层构造及合并检查。
    fn toml(text: &str) -> Result<Configuration, ConfigError> {
        let dir = TestDir::new();
        let path = dir.write("input.toml", text);
        Configuration::builder()
            .add_source(nestrs_config::sources::Toml::file(path))
            .build()
    }

    #[cfg(feature = "dotenv")]
    // 通过正式 DotEnv 文件来源加载固定前缀，保持对真实加载流程的覆盖。
    fn dotenv(text: &str) -> Result<Configuration, ConfigError> {
        let dir = TestDir::new();
        let path = dir.write("input.env", text);
        Configuration::builder()
            .add_source(nestrs_config::sources::DotEnv::file(path).prefix("A__"))
            .build()
    }

    #[cfg(feature = "json")]
    #[test]
    // 覆盖超过 f64 精确整数范围的值、整数两端边界、浮点及 null，确认文件字符串不自动转数值。
    fn json_preserves_large_integers_and_typed_strings() {
        let config = json(
            r#"{"large":9007199254740993,"max":18446744073709551615,"min":-9223372036854775808,"text":"42","decimal":1.25,"nil":null}"#,
        )
        .unwrap();
        assert_eq!(
            config.get_required::<u64>("large").unwrap(),
            9007199254740993
        );
        assert_eq!(config.get_required::<u64>("max").unwrap(), u64::MAX);
        assert_eq!(config.get_required::<i64>("min").unwrap(), i64::MIN);
        assert_eq!(config.get_required::<f64>("decimal").unwrap(), 1.25);
        assert_eq!(config.get_required::<Option<u8>>("nil").unwrap(), None);
        assert!(config.get_required::<u64>("text").is_err());
    }

    #[cfg(feature = "json")]
    #[test]
    // 在根、嵌套对象和数组元素中检查重复键，包含转义后相同的键，避免 Map 覆盖掩盖问题。
    fn json_rejects_duplicate_keys_at_every_level() {
        for text in [
            r#"{"key":1,"key":2}"#,
            r#"{"nested":{"key":1,"key":2}}"#,
            r#"{"items":[{"a":1,"\u0061":2}]}"#,
        ] {
            let error = json(text).unwrap_err();
            assert_eq!(error.kind(), ConfigErrorKind::DuplicateKey);
            assert!(error.origin().is_some());
        }
    }

    #[cfg(feature = "json")]
    #[test]
    // 验证整数/浮点越界属于不支持值，并要求配置文件根必须是对象。
    fn json_rejects_numeric_overflow_and_nonobject_roots() {
        for text in [
            "{\"x\":18446744073709551616}",
            "{\"x\":-9223372036854775809}",
            "{\"x\":1e9999}",
        ] {
            assert_eq!(
                json(text).unwrap_err().kind(),
                ConfigErrorKind::UnsupportedValue
            );
        }
        for text in ["null", "[]", "1", "\"text\""] {
            assert_eq!(
                json(text).unwrap_err().kind(),
                ConfigErrorKind::TypeMismatch
            );
        }
    }

    #[cfg(feature = "json")]
    #[test]
    // 核对成功节点的实际行列，并用含秘密的非法 JSON 检查错误不会附带原始片段。
    fn json_origins_use_real_value_locations_and_errors_hide_content() {
        let config = json("{\n  \"port\": 3000,\n  \"host\": \"localhost\"\n}").unwrap();
        let explanation = config.explain("port").unwrap();
        let origin = explanation.origins().single().unwrap();
        assert_eq!(origin.line(), Some(2));
        assert_eq!(origin.column(), Some(11));
        let secret = "raw-json-secret-8166";
        let error = json(&format!("{{\"x\":\"{secret}\" trailing}}")).unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Parse);
        assert_safe(&error, secret);
    }

    #[cfg(feature = "json")]
    #[test]
    // 分别耗尽嵌套深度和累计节点预算，确认两类结构压力都归为资源限制。
    fn json_rejects_depth_and_node_exhaustion() {
        let depth = nestrs_config::MAX_DEPTH + 1;
        let text = format!("{{\"x\":{}0{}}}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(json(&text).unwrap_err().kind(), ConfigErrorKind::Limit);
        let text = format!(
            "{{\"x\":[{}]}}",
            vec!["0"; nestrs_config::MAX_NODES].join(",")
        );
        assert_eq!(json(&text).unwrap_err().kind(), ConfigErrorKind::Limit);
    }

    #[cfg(feature = "json")]
    #[test]
    // 验证固定基础目录、缺失可选文件和不可变快照；坏语法、目录及非 UTF-8 不得被 optional 吞掉。
    fn optional_files_ignore_only_absence_and_obey_base_dir() {
        use nestrs_config::sources::Json;
        let dir = TestDir::new();
        dir.write("exists.json", r#"{"key":"before"}"#);
        let config = Configuration::builder()
            .base_dir(dir.path())
            .add_source(Json::file("absent.json").optional())
            .add_source(Json::file("exists.json"))
            .build()
            .unwrap();
        dir.write("exists.json", r#"{"key":"after"}"#);
        assert_eq!(
            config.clone().get_required::<String>("key").unwrap(),
            "before"
        );
        dir.write("bad.json", "not JSON");
        let error = Configuration::builder()
            .add_source(Json::file(dir.path().join("bad.json")).optional())
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Parse);
        let error = Configuration::builder()
            .add_source(Json::file(dir.path()).optional())
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Io);
        let error = Configuration::builder()
            .add_source(Json::file(dir.write("invalid-utf8.json", [0xff])))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Encoding);
    }

    #[cfg(feature = "json")]
    #[test]
    // 用超限文件验证读取预算先于解析生效，避免把文件长度耗尽误报为 JSON 语法错误。
    fn oversized_files_fail_before_parsing() {
        let dir = TestDir::new();
        let path = dir.path().join("large.json");
        std::fs::File::create(&path)
            .unwrap()
            .set_len(nestrs_config::MAX_FILE_BYTES as u64 + 1)
            .unwrap();
        let error = Configuration::builder()
            .add_source(nestrs_config::sources::Json::file(path))
            .build()
            .unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Limit);
    }

    #[cfg(all(unix, feature = "json"))]
    #[test]
    // 在子进程尝试读取静态 FIFO；父进程超时可终止旧的阻塞行为，避免整个测试进程挂起。
    fn static_fifo_is_rejected_without_blocking() {
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let dir = TestDir::new();
        let fifo = dir.path().join("input.fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );

        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "files::fifo_read_child", "--nocapture"])
            .env("NESTRS_CONFIG_FIFO_CHILD_PATH", &fifo)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if child.try_wait().unwrap().is_some() {
                let output = child.wait_with_output().unwrap();
                assert!(
                    output.status.success(),
                    "FIFO child failed: {}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                break;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                let _ = child.wait();
                panic!("opening a static FIFO blocked the configuration source");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[cfg(all(unix, feature = "json"))]
    #[test]
    // FIFO 子进程入口，分别检查必选与 optional 来源都拒绝非普通文件；常规测试运行不执行读取。
    fn fifo_read_child() {
        let Some(path) = std::env::var_os("NESTRS_CONFIG_FIFO_CHILD_PATH") else {
            return;
        };
        for optional in [false, true] {
            let source = nestrs_config::sources::Json::file(PathBuf::from(&path));
            let source = if optional { source.optional() } else { source };
            let error = Configuration::builder()
                .add_source(source)
                .build()
                .unwrap_err();
            assert_eq!(error.kind(), ConfigErrorKind::Io);
        }
    }

    #[cfg(feature = "toml")]
    #[test]
    // 验证 TOML 进制、大整数、字符串类型以及节点 span 对应的真实行列。
    fn toml_preserves_radices_integer_precision_and_locations() {
        let config = toml(
            "[database]\nport = 5432\nlarge = 9007199254740993\nmax = 18446744073709551615\nhex = 0xff\ntext = \"42\"\n",
        )
        .unwrap();
        assert_eq!(
            config.get_required::<u64>("database.large").unwrap(),
            9007199254740993
        );
        assert_eq!(
            config.get_required::<u64>("database.max").unwrap(),
            u64::MAX
        );
        assert_eq!(config.get_required::<u16>("database.hex").unwrap(), 255);
        assert!(config.get_required::<u8>("database.text").is_err());
        let explanation = config.explain("database.port").unwrap();
        let origin = explanation.origins().single().unwrap();
        assert_eq!(origin.line(), Some(2));
        assert_eq!(origin.column(), Some(8));
    }

    #[cfg(feature = "toml")]
    #[test]
    // 验证共同值域明确排除 TOML 日期时间、非有限数和溢出整数，并拒绝多种重复声明。
    fn toml_rejects_dates_nonfinite_numbers_and_duplicates() {
        for text in [
            "date = 2026-10-04",
            "number = nan",
            "number = +inf",
            "number = -inf",
            "number = 18446744073709551616",
        ] {
            assert_eq!(
                toml(text).unwrap_err().kind(),
                ConfigErrorKind::UnsupportedValue
            );
        }
        for text in ["x = 1\nx = 2", "x = { y = 1, y = 2 }", "[x]\na=1\n[x]\nb=2"] {
            assert_eq!(
                toml(text).unwrap_err().kind(),
                ConfigErrorKind::DuplicateKey
            );
        }
    }

    #[cfg(feature = "toml")]
    #[test]
    // 检查 TOML 错误不泄漏原始行，同时覆盖转换阶段的配置深度预算。
    fn toml_errors_hide_raw_lines_and_reject_excessive_depth() {
        let secret = "raw-toml-secret-bc67";
        let error = toml(&format!("x = \"{secret}\" trailing")).unwrap_err();
        assert_eq!(error.kind(), ConfigErrorKind::Parse);
        assert_safe(&error, secret);
        let depth = nestrs_config::MAX_DEPTH + 1;
        let text = format!("x = {}0{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(toml(&text).unwrap_err().kind(), ConfigErrorKind::Limit);
    }

    #[cfg(feature = "toml")]
    #[test]
    // 用数组和内联表触发原生解析器自身的递归保护，验证其固定错误消息被归类为 Limit。
    fn toml_native_parser_recursion_limits_are_resource_errors() {
        for depth in [127, 128, 1_000] {
            let array = format!("x = {}0{}", "[".repeat(depth), "]".repeat(depth));
            assert_eq!(
                toml(&array).unwrap_err().kind(),
                ConfigErrorKind::Limit,
                "array depth {depth}"
            );
            let table = format!("x = {}0{}", "{ x = ".repeat(depth), " }".repeat(depth));
            assert_eq!(
                toml(&table).unwrap_err().kind(),
                ConfigErrorKind::Limit,
                "inline table depth {depth}"
            );
        }
    }

    #[cfg(all(feature = "toml", feature = "json"))]
    #[test]
    // 组合 TOML、JSON 和环境来源，验证后层覆盖标量、对象按键保留及空数组整体替换。
    fn files_merge_in_order_and_environment_overrides_scalars() {
        use nestrs_config::sources::{Json, Toml};
        let dir = TestDir::new();
        let toml_path = dir.write(
            "base.toml",
            "[app]\nhost=\"base\"\nport=5432\nitems=[1,2]\n",
        );
        let json_path = dir.write("override.json", r#"{"app":{"host":"prod","items":[]}}"#);
        let config = Configuration::builder()
            .add_source(Toml::file(toml_path))
            .add_source(Json::file(json_path))
            .add_source(Environment::from_iter("A__", [("A__APP__PORT", "6432")]))
            .build()
            .unwrap();
        assert_eq!(config.get_required::<String>("app.host").unwrap(), "prod");
        assert_eq!(config.get_required::<u16>("app.port").unwrap(), 6432);
        assert!(
            config
                .get_required::<Vec<u8>>("app.items")
                .unwrap()
                .is_empty()
        );
    }

    #[cfg(feature = "dotenv")]
    #[test]
    // 验证 export、引号、有限转义、注释和空值，并确认变量/命令样式保持字面文本及来源行号。
    fn dotenv_handles_quotes_comments_and_text_without_expansion() {
        let config = dotenv(
            "# ignored\nexport A__PORT = 3000 # comment\nA__SINGLE=' literal $HOME \\n'\nA__DOUBLE=\"a\\nb\\t\\\"\\\\\"\nA__BARE=$HOME $(echo hello)\nA__HASH=part#literal\nA__EMPTY=\nA__NULL=null\n",
        )
        .unwrap();
        assert_eq!(config.get_required::<u16>("port").unwrap(), 3000);
        assert_eq!(
            config.get_required::<String>("single").unwrap(),
            " literal $HOME \\n"
        );
        assert_eq!(
            config.get_required::<String>("double").unwrap(),
            "a\nb\t\"\\"
        );
        assert_eq!(
            config.get_required::<String>("bare").unwrap(),
            "$HOME $(echo hello)"
        );
        assert_eq!(
            config.get_required::<String>("hash").unwrap(),
            "part#literal"
        );
        assert_eq!(config.get_required::<String>("empty").unwrap(), "");
        assert_eq!(
            config.get_required::<Option<String>>("null").unwrap(),
            Some("null".into())
        );
        let explanation = config.explain("port").unwrap();
        assert_eq!(explanation.origins().single().unwrap().line(), Some(2));
    }

    #[cfg(feature = "dotenv")]
    #[test]
    // 验证非法赋值、未闭合引号、未知转义、多行/续行和同层冲突被拒绝，错误不输出秘密。
    fn dotenv_rejects_malformed_lines_duplicates_and_continuations() {
        for text in [
            "A__X",
            "A__X='unterminated",
            "A__X=\"unsupported\\q\"",
            "A__X='x' trailing",
            "A__X=line\\\nnext",
            "A__X=\"first\nsecond\"",
            "1INVALID=x",
        ] {
            assert_eq!(dotenv(text).unwrap_err().kind(), ConfigErrorKind::Parse);
        }
        for text in ["A__X=1\nA__X=2", "A__X=1\nA__x=2", "A__X=1\nA__X__Y=2"] {
            assert_eq!(
                dotenv(text).unwrap_err().kind(),
                ConfigErrorKind::DuplicateKey
            );
        }
        let secret = "raw-dotenv-secret-523e";
        assert_safe(&dotenv(&format!("A__X='{secret}")).unwrap_err(), secret);
    }
}
