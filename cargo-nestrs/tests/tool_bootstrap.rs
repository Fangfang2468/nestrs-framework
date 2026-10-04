//! 工具源码可在自身 IDE 准备中使用限定的 bootstrap；普通应用及同名伪装目标不获授权。
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde_json::Value;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nestrs tool bootstrap 空格 {}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn command(&self, operation: &str, manifest: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        clear_build_overrides(&mut command);
        command
            .arg(operation)
            .args(["--offline", "--manifest-path"])
            .arg(manifest)
            .current_dir(&self.0)
            .env("CARGO_TARGET_DIR", self.0.join("target 构建"))
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"));
        command
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn clear_build_overrides(command: &mut Command) {
    for name in [
        "RUSTC_BOOTSTRAP",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTDOC",
        "NESTRS_REAL_RUSTDOC",
        "NESTRS_COMPILER_OUTPUT",
        "NESTRS_IDE_CAPTURE",
        "NESTRS_IDE_CONSTRUCTORS",
        "NESTRS_GRAPH_TARGET",
        "NESTRS_GRAPH_BINARY",
        "NESTRS_GRAPH_MANIFEST",
        "NESTRS_GRAPH_PROOF",
        "NESTRS_GRAPH_SOURCE",
        "NESTRS_GRAPH_PLAN",
    ] {
        command.env_remove(name);
    }
}

fn success(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

#[test]
fn own_bridge_ide_units_and_driver_check_receive_only_their_required_bootstrap() {
    let workspace = Workspace::new();
    let tool = Path::new(env!("CARGO_MANIFEST_DIR"));
    let bridge = tool.join("internal/bridge");
    let model = workspace.0.join("editor/rust-project.json");
    // Select only the real bridge package, retaining init's default --all-targets.
    // --vscode is intentionally absent: this must not edit the source checkout.
    success(
        workspace
            .command("init", &bridge.join("Cargo.toml"))
            .args(["--locked", "-p", "nestrs-tool-bridge", "--output"])
            .arg(&model),
    );
    let model_bytes = fs::read(&model).unwrap();
    let project: Value = serde_json::from_slice(&model_bytes).unwrap();
    let bridge_source = bridge.join("src/lib.rs").canonicalize().unwrap();
    let units: Vec<_> = project["crates"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|unit| {
            unit["display_name"] == "nestrs_tool_bridge"
                && Path::new(unit["root_module"].as_str().unwrap())
                    .canonicalize()
                    .unwrap()
                    == bridge_source
        })
        .collect();
    assert_eq!(units.len(), 2, "bridge library and test units: {units:?}");
    for test in [false, true] {
        assert!(units.iter().any(|unit| {
            unit["cfg"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cfg| cfg == "test")
                == test
        }));
    }
    for unit in project["crates"].as_array().unwrap() {
        assert!(unit["env"].get("RUSTC_BOOTSTRAP").is_none(), "{unit}");
    }

    let settings_path = model.with_file_name("rust-analyzer-settings.json");
    let settings_bytes = fs::read(&settings_path).unwrap();
    let settings: Value = serde_json::from_slice(&settings_bytes).unwrap();
    let check = settings["rust-analyzer.check.overrideCommand"]
        .as_array()
        .unwrap();
    assert_eq!(check[1], "init");
    assert_eq!(check[2], "check");
    let mut command = Command::new(check[0].as_str().unwrap());
    clear_build_overrides(&mut command);
    command
        .args(check[1..].iter().map(|arg| arg.as_str().unwrap()))
        .current_dir(&workspace.0);
    for (name, value) in settings["rust-analyzer.check.extraEnv"]
        .as_object()
        .unwrap()
    {
        assert_ne!(name, "RUSTC_BOOTSTRAP");
        command.env(name, value.as_str().unwrap());
    }
    let checked = success(&mut command);
    assert!(
        String::from_utf8_lossy(&checked.stdout)
            .lines()
            .any(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|message| {
                    message["reason"] == "build-finished" && message["success"] == true
                })
            })
    );
    assert_eq!(fs::read(&model).unwrap(), model_bytes);
    assert_eq!(fs::read(&settings_path).unwrap(), settings_bytes);

    success(workspace.command("check", &tool.join("Cargo.toml")).args([
        "--locked",
        "-p",
        "cargo-nestrs",
        "--features",
        "compiler-driver",
        "--bin",
        "nestrs-driver",
    ]));
}

#[test]
fn application_build_scripts_and_same_named_crates_stay_unprivileged() {
    let workspace = Workspace::new();
    fs::write(
        workspace.0.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"ordinary\", \"bridge\", \"driver\"]\n",
    )
    .unwrap();
    for member in ["ordinary", "bridge", "driver"] {
        fs::create_dir_all(workspace.0.join(member).join("src")).unwrap();
    }
    fs::write(
        workspace.0.join("ordinary/Cargo.toml"),
        "[package]\nname = \"ordinary\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        workspace.0.join("ordinary/build.rs"),
        r#"fn main() {
    assert!(std::env::var_os("RUSTC_BOOTSTRAP").is_none());
    println!("cargo:rustc-env=NESTRS_BOOTSTRAP_BUILD_SCRIPT=clean");
}
"#,
    )
    .unwrap();
    fs::write(
        workspace.0.join("ordinary/src/lib.rs"),
        r#"const _: () = match option_env!("RUSTC_BOOTSTRAP") {
    None => (),
    Some(_) => panic!("application inherited bootstrap"),
};
pub const BUILD_SCRIPT_RAN: &str = env!("NESTRS_BOOTSTRAP_BUILD_SCRIPT");
"#,
    )
    .unwrap();
    fs::write(
        workspace.0.join("bridge/Cargo.toml"),
        "[package]\nname = \"nestrs-tool-bridge\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[lib]\nname = \"nestrs_tool_bridge\"\nproc-macro = true\n",
    )
    .unwrap();
    fs::write(
        workspace.0.join("bridge/src/lib.rs"),
        "#![feature(proc_macro_def_site)]\n",
    )
    .unwrap();
    fs::write(
        workspace.0.join("driver/Cargo.toml"),
        "[package]\nname = \"cargo-nestrs\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[[bin]]\nname = \"nestrs-driver\"\npath = \"src/bin/nestrs-driver.rs\"\n",
    )
    .unwrap();
    fs::create_dir(workspace.0.join("driver/src/bin")).unwrap();
    fs::write(
        workspace.0.join("driver/src/bin/nestrs-driver.rs"),
        "#![feature(proc_macro_def_site)]\nfn main() {}\n",
    )
    .unwrap();
    let manifest = workspace.0.join("Cargo.toml");
    success(
        workspace
            .command("check", &manifest)
            .args(["-p", "ordinary", "--all-targets"])
            .env("RUSTC_BOOTSTRAP", "1"),
    );

    let model = workspace.0.join("editor/rust-project.json");
    let suggested = model.with_file_name("rust-analyzer-settings.json");
    let vscode = workspace.0.join(".vscode/settings.json");
    fs::create_dir_all(model.parent().unwrap()).unwrap();
    fs::create_dir_all(vscode.parent().unwrap()).unwrap();
    let old_model = b"{\"crates\":[],\"sentinel\":\"old project\"}\n";
    let old_suggested = b"{\"sentinel\":\"old suggested settings\"}\n";
    let old_vscode = b"{\"editor.tabSize\":3,\"sentinel\":\"old user settings\"}\n";
    fs::write(&model, old_model).unwrap();
    fs::write(&suggested, old_suggested).unwrap();
    fs::write(&vscode, old_vscode).unwrap();
    for package in ["nestrs-tool-bridge", "cargo-nestrs"] {
        let output = workspace
            .command("init", &manifest)
            .args(["--vscode", "-p", package, "--output"])
            .arg(&model)
            .env("RUSTC_BOOTSTRAP", "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{package}: {stderr}");
        assert!(stderr.contains("E0554"), "{package}: {stderr}");
        assert!(
            stderr.contains("proc_macro_def_site"),
            "{package}: {stderr}"
        );
        assert!(
            stderr.contains("the previous editor project and settings were preserved"),
            "{package}: {stderr}",
        );
        assert_eq!(fs::read(&model).unwrap(), old_model);
        assert_eq!(fs::read(&suggested).unwrap(), old_suggested);
        assert_eq!(fs::read(&vscode).unwrap(), old_vscode);
    }
}
