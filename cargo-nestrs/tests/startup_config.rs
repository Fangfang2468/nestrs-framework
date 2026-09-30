//! 用真实 CLI、编译器和运行时验证项目启动配置，避免只证明 TOML 能被解析。
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

struct Fixture(PathBuf);

impl Fixture {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("target/startup-config")
            .join(std::process::id().to_string())
            .join(name);
        for directory in ["shared/src", "lazy-app/src", "eager-app/src", "export"] {
            fs::create_dir_all(root.join(directory)).unwrap();
        }
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = ['shared', 'lazy-app', 'eager-app']\nresolver = '3'\n",
        )
        .unwrap();
        let core = serde_json::to_string(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("nestrs-core"),
        )
        .unwrap();
        fs::write(
            root.join("shared/Cargo.toml"),
            format!(
                "[package]\nname = 'startup-shared'\nversion = '0.0.0'\nedition = '2024'\n\
                 [dependencies]\nnestrs-core = {{ path = {core} }}\n\
                 tokio = {{ version = '1.53.1', features = ['rt', 'macros', 'sync', 'time'] }}\n\
                 [nestrs-cli]\ninitialization = 'eager'\nmax-concurrent-activations = 7\n"
            ),
        )
        .unwrap();
        fs::write(root.join("shared/src/lib.rs"), SHARED_SOURCE).unwrap();
        let fixture = Self(root);
        for (package, eager, limit) in [("lazy-app", false, 1), ("eager-app", true, 2)] {
            fixture.manifest(package, Some((eager, limit)));
            fs::write(
                fixture.0.join(package).join("src/main.rs"),
                format!(
                    "{APPLICATION_SOURCE}\n\
                     #[tokio::test(flavor = \"current_thread\")]\n\
                     async fn test_entry_uses_its_own_package_defaults() {{\n\
                     startup_shared::verify({eager}, {limit}, None).await;\n}}\n"
                ),
            )
            .unwrap();
        }
        fixture
    }

    fn manifest(&self, package: &str, config: Option<(bool, usize)>) {
        let config = config.map_or_else(String::new, |(eager, limit)| {
            format!(
                "\n[nestrs-cli]\ninitialization = '{}'\nmax-concurrent-activations = {limit}\n",
                if eager { "eager" } else { "lazy" },
            )
        });
        self.manifest_text(package, &config);
    }

    fn manifest_text(&self, package: &str, config: &str) {
        fs::write(
            self.0.join(package).join("Cargo.toml"),
            format!(
                "[package]\nname = '{package}'\nversion = '0.0.0'\nedition = '2024'\n\
                 [dependencies]\nstartup-shared = {{ path = '../shared' }}\n\
                 tokio = {{ version = '1.53.1', features = ['rt', 'macros'] }}\n{config}"
            ),
        )
        .unwrap();
    }

    fn cli(&self, arguments: &[&str], log: &str) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .args(arguments)
            .current_dir(&self.0)
            .env("CARGO_TARGET_DIR", self.0.join("build"))
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"));
        for name in [
            "CARGO_ENCODED_RUSTFLAGS",
            "RUSTFLAGS",
            "RUSTC_BOOTSTRAP",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
            "RUSTDOC",
            "NESTRS_REAL_RUSTDOC",
            "NESTRS_GRAPH_TARGET",
            "NESTRS_IDE_CAPTURE",
        ] {
            command.env_remove(name);
        }
        if arguments.first() == Some(&"graph") {
            command.env(
                "NESTRS_STARTUP_FACTORY_SENTINEL",
                self.0.join("graph-executed-factory"),
            );
        }
        let output = command.output().unwrap();
        self.log(log, &output);
        output
    }

    fn log(&self, name: &str, output: &Output) {
        fs::write(
            self.0.join(format!("{name}.log")),
            format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            ),
        )
        .unwrap();
    }

    fn build(&self, package: &str, log: &str) -> PathBuf {
        let output = self.cli(
            &["build", "--offline", "-p", package, "--message-format=json"],
            log,
        );
        assert_success(&output);
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|message| {
                (message["reason"] == "compiler-artifact" && message["target"]["name"] == package)
                    .then(|| message["executable"].as_str().map(PathBuf::from))
                    .flatten()
            })
            .expect("Cargo 必须返回当前 binary 的可执行文件路径")
    }

    fn execute(&self, executable: &Path, arguments: &[&str], log: &str) {
        let output = Command::new(executable)
            .args(arguments)
            .current_dir(self.0.join("export"))
            .output()
            .unwrap();
        self.log(log, &output);
        assert_success(&output);
        assert!(String::from_utf8_lossy(&output.stdout).contains("startup configuration verified"));
    }
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn package_defaults_are_frozen_per_entry_and_explicit_options_override_them() {
    let fixture = Fixture::new("runtime");
    let lazy = fixture.build("lazy-app", "build-lazy");
    let eager = fixture.build("eager-app", "build-eager");

    // 两个 binary 共享含有 build 调用的同一个 rlib，但使用各自入口的配置。
    // shared 自身配置为 eager / 7，不能渗透到 lazy / 1 或 eager / 2 的宿主。
    fixture.execute(&lazy, &["lazy", "1"], "run-lazy");
    fixture.execute(&eager, &["eager", "2"], "run-eager");
    fixture.execute(&lazy, &["eager", "2", "eager", "2"], "override-eager");
    fixture.execute(&eager, &["lazy", "1", "lazy", "1"], "override-lazy");
    fixture.execute(&eager, &["lazy", "32", "defaults"], "override-defaults");

    // 真正的 test 入口也必须带上所属 package 的配置，不能沿用上次 binary。
    assert_success(&fixture.cli(
        &["test", "--offline", "--workspace", "--bins"],
        "test-entries",
    ));
    // doctest 的入口属于 shared，因此使用 shared 的 eager / 7。
    assert_success(&fixture.cli(
        &["test", "--offline", "-p", "startup-shared", "--doc"],
        "doctest-entry",
    ));

    // 只改 Cargo.toml，源码和 target 不变。两次都验证实际构造行为，以捕获缓存未失效。
    fixture.manifest("lazy-app", Some((true, 2)));
    let changed = fixture.build("lazy-app", "rebuild-config-only");
    fixture.execute(&changed, &["eager", "2"], "run-changed-config");
    fixture.manifest("lazy-app", None);
    let removed = fixture.build("lazy-app", "rebuild-config-removed");
    fixture.execute(&removed, &["lazy", "32"], "run-removed-config");

    // 已构建产物可独立分发。移走所有 fixture manifest 后，从无 manifest 的目录执行。
    let exported = fixture
        .0
        .join("export")
        .join(format!("standalone{}", std::env::consts::EXE_SUFFIX));
    fs::copy(&eager, &exported).unwrap();
    let manifests = ["", "lazy-app", "eager-app", "shared"]
        .map(|package| fixture.0.join(package).join("Cargo.toml"));
    for manifest in &manifests {
        fs::rename(manifest, manifest.with_extension("saved")).unwrap();
    }
    let output = Command::new(&exported)
        .args(["eager", "2"])
        .current_dir(fixture.0.join("export"))
        .output();
    // 无论子进程是否成功，恢复 fixture，便于继续复现问题。
    for manifest in &manifests {
        fs::rename(manifest.with_extension("saved"), manifest).unwrap();
    }
    let output = output.unwrap();
    fixture.log("standalone-without-manifests", &output);
    assert_success(&output);
}

#[test]
fn graph_and_ide_accept_configuration_and_check_rejects_invalid_values() {
    let fixture = Fixture::new("tool-paths");
    // graph 当前要求 binary 直接依赖 core；仅此 fixture 满足该现有边界。
    // 上面的 runtime fixture 继续保留间接依赖，验证共享库中的 build 入口。
    let manifest = fixture.0.join("eager-app/Cargo.toml");
    let core = serde_json::to_string(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("nestrs-core"),
    )
    .unwrap();
    fs::write(
        &manifest,
        fs::read_to_string(&manifest).unwrap().replace(
            "[dependencies]\n",
            &format!("[dependencies]\nnestrs-core = {{ path = {core} }}\n"),
        ),
    )
    .unwrap();
    let graph = fixture.0.join("graph.html");
    assert_success(&fixture.cli(
        &[
            "graph",
            "--offline",
            "-p",
            "eager-app",
            "--bin",
            "eager-app",
            "--output",
            graph.to_str().unwrap(),
        ],
        "graph",
    ));
    assert!(fs::read_to_string(graph).unwrap().contains("graph-data"));
    assert!(
        !fixture.0.join("graph-executed-factory").exists(),
        "Eager 配置不能让 graph 诊断入口执行 factory",
    );
    let model = fixture.0.join("rust-project.json");
    assert_success(&fixture.cli(
        &[
            "init",
            "check",
            "--offline",
            "-p",
            "eager-app",
            "--output",
            model.to_str().unwrap(),
        ],
        "init-check",
    ));
    let model: Value = serde_json::from_slice(&fs::read(model).unwrap()).unwrap();
    assert!(!model["crates"].as_array().unwrap().is_empty());

    for (name, setting, field) in [
        (
            "zero",
            "max-concurrent-activations = 0",
            "max-concurrent-activations",
        ),
        ("mode", "initialization = 'sometimes'", "initialization"),
        (
            "unknown",
            "max_concurrent_activations = 2",
            "max_concurrent_activations",
        ),
    ] {
        fixture.manifest_text("eager-app", &format!("[nestrs-cli]\n{setting}\n"));
        let output = fixture.cli(&["check", "--offline", "-p", "eager-app"], name);
        assert!(!output.status.success(), "CLI 接受了非法配置：{setting}",);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains(&format!("nestrs-cli.{field}")),
            "必须指出实际配置字段，不能只因普通编译错误失败：{stderr}",
        );
    }
}

const APPLICATION_SOURCE: &str = r#"
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let eager = args[0] == "eager";
    let limit: usize = args[1].parse().unwrap();
    let explicit = match args.get(2).map(String::as_str) {
        None => None,
        Some("defaults") => Some(startup_shared::default_options()),
        Some(mode) => Some(startup_shared::options(mode == "eager", args[3].parse().unwrap())),
    };
    startup_shared::verify(eager, limit, explicit).await;
    println!("startup configuration verified");
}
"#;

const SHARED_SOURCE: &str = r#"
//! 此 doctest 属于库 package，自身 eager / 7 必须应用到生成的测试入口。
//! ```
//! #[tokio::main(flavor = "current_thread")]
//! async fn main() {
//!     startup_shared::verify(true, 7, None).await;
//! }
//! ```
use std::sync::atomic::{AtomicUsize, Ordering};
use nestrs::factory;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use tokio::sync::{Notify, Semaphore};

static STARTED: AtomicUsize = AtomicUsize::new(0);
static FINISHED: AtomicUsize = AtomicUsize::new(0);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static EVENTS: Notify = Notify::const_new();
static GATE: Semaphore = Semaphore::const_new(0);

struct First;
struct Second;
struct Third;

async fn construct() {
    if let Some(path) = std::env::var_os("NESTRS_STARTUP_FACTORY_SENTINEL") {
        std::fs::write(path, "factory executed").unwrap();
    }
    let active = ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
    PEAK.fetch_max(active, Ordering::SeqCst);
    STARTED.fetch_add(1, Ordering::SeqCst);
    EVENTS.notify_one();
    GATE.acquire().await.unwrap().forget();
    ACTIVE.fetch_sub(1, Ordering::SeqCst);
    FINISHED.fetch_add(1, Ordering::SeqCst);
    EVENTS.notify_one();
}

#[factory(lifetime = Singleton)]
async fn first() -> First { construct().await; First }
#[factory(lifetime = Singleton)]
async fn second() -> Second { construct().await; Second }
#[factory(lifetime = Singleton)]
async fn third() -> Third { construct().await; Third }

pub fn default_options() -> ServiceProviderOptions { ServiceProviderOptions::default() }

pub fn options(eager: bool, limit: usize) -> ServiceProviderOptions {
    ServiceProviderOptions {
        initialization: if eager { InitializationMode::Eager } else { InitializationMode::Lazy },
        max_concurrent_activations: std::num::NonZeroUsize::new(limit).unwrap(),
    }
}

async fn wait_count(counter: &AtomicUsize, expected: usize) {
    loop {
        let notification = EVENTS.notified();
        if counter.load(Ordering::SeqCst) >= expected { return; }
        notification.await;
    }
}

pub async fn verify(eager: bool, limit: usize, explicit: Option<ServiceProviderOptions>) {
    let defaults = ServiceProviderOptions::default();
    assert_eq!(defaults.initialization, InitializationMode::Lazy);
    assert_eq!(defaults.max_concurrent_activations.get(), 32);

    let activate = async {
        // build 刻意位于共享库中，配置必须来自最终入口而非当前函数所属 package。
        let provider = match explicit {
            Some(options) => ServiceProvider::build_with_options(options).await,
            None => ServiceProvider::build().await,
        }.unwrap();
        assert_eq!(STARTED.load(Ordering::SeqCst), if eager { 3 } else { 0 });
        let (first, second, third) = tokio::join!(
            nestrs_core::get_required_service!(provider, First),
            nestrs_core::get_required_service!(provider, Second),
            nestrs_core::get_required_service!(provider, Third),
        );
        first.unwrap(); second.unwrap(); third.unwrap();
        provider.dispose_async().await.unwrap();
    };
    let release = async {
        let mut released = 0;
        while released < 3 {
            let batch = limit.min(3 - released);
            wait_count(&STARTED, released + batch).await;
            // 每个 factory 都在显式 gate 上暂停；让其它已就绪 worker 获得轮询机会。
            // 无随机 sleep。若错误地只准许一个 worker，双并发批次无法进入下一步。
            tokio::task::yield_now().await;
            assert_eq!(STARTED.load(Ordering::SeqCst), released + batch);
            assert_eq!(ACTIVE.load(Ordering::SeqCst), batch);
            GATE.add_permits(batch);
            released += batch;
            wait_count(&FINISHED, released).await;
        }
    };
    // 超时仅防止调度回归让测试永久挂起；正确性由计数和 gate 协议证明。
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::join!(activate, release);
    }).await.expect("configured activation concurrency did not make progress");
    assert_eq!(STARTED.load(Ordering::SeqCst), 3);
    assert_eq!(FINISHED.load(Ordering::SeqCst), 3);
    assert_eq!(PEAK.load(Ordering::SeqCst), limit.min(3));
}
"#;
