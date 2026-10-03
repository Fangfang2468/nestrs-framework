//! Run the real tools on each supported native host, including Windows MSVC.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

struct Workspace(PathBuf);

impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("nestrs native 空格 {}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn fixture(&self, name: &str) -> PathBuf {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let source = crate_root.join("tests/fixtures").join(name);
        let destination = self.0.join(name);
        copy_directory(&source, &destination);
        let manifest = destination.join("Cargo.toml");
        let core = crate_root.parent().unwrap().join("nestrs-core");
        let text = fs::read_to_string(&manifest).unwrap();
        assert!(text.contains("\"../../../../nestrs-core\""));
        rewrite_core_dependencies(&destination, &serde_json::to_string(&core).unwrap());
        manifest
    }

    fn command(&self, operation: &str, manifest: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .arg(operation)
            .args(["--locked", "--offline", "--manifest-path"])
            .arg(manifest)
            .env("CARGO_TARGET_DIR", self.0.join("target 构建"))
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"));
        clear_build_overrides(&mut command);
        command
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "target" || entry.file_name() == ".vscode" {
            continue;
        }
        let output = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&entry.path(), &output);
        } else {
            fs::copy(entry.path(), output).unwrap();
        }
    }
}

fn rewrite_core_dependencies(directory: &Path, core: &str) {
    for entry in fs::read_dir(directory).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            rewrite_core_dependencies(&entry.path(), core);
        } else if entry.file_name() == "Cargo.toml" {
            let text = fs::read_to_string(entry.path()).unwrap();
            fs::write(
                entry.path(),
                text.replace("\"../../../../nestrs-core\"", core)
                    .replace("\"../../../../../nestrs-core\"", core),
            )
            .unwrap();
        }
    }
}

fn graph_data(path: &Path) -> Value {
    let html = fs::read_to_string(path).unwrap();
    let script = html.split_once("id=\"graph-data\"").unwrap().1;
    let json = script.split_once('>').unwrap().1;
    serde_json::from_str(json.split_once("</script>").unwrap().0).unwrap()
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
        "NESTRS_GRAPH_TARGET",
        "NESTRS_IDE_CAPTURE",
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

/// JSON 诊断可在没有项目和 Cargo 的目录工作，且显式目录查询只读取安装状态。
#[test]
fn doctor_reports_real_artifacts_and_cache_paths_without_a_project_or_writes() {
    use cargo_nestrs::toolchain::{CompilerIdentity, Toolchain};

    let directory = std::env::temp_dir().join(format!("nestrs doctor 空格 {}", std::process::id()));
    fs::create_dir(&directory).unwrap();
    let workspace = Workspace(directory.canonicalize().unwrap());
    let doctor = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .arg("doctor")
            .current_dir(&workspace.0)
            .env("CARGO", workspace.0.join("cargo-must-not-run"))
            .env("CARGO_TARGET_DIR", workspace.0.join("ignored-target"))
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"));
        clear_build_overrides(&mut command);
        command
    };

    let output = success(doctor().arg("--json"));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["version"], 1);
    let identity = CompilerIdentity::pinned().unwrap();
    assert_eq!(report["rustc"]["release"], identity.release);
    assert_eq!(report["rustc"]["commit_hash"], identity.commit);
    assert_eq!(report["rustc"]["host"], identity.host);
    for field in ["compiler", "sysroot", "driver", "macro_bridge"] {
        let path = Path::new(report[field].as_str().unwrap());
        assert!(path.is_absolute() && path.exists(), "{field}: {path:?}");
    }
    for field in [
        "target_directory",
        "cache_directory",
        "compiler_output_directory",
    ] {
        assert!(report.get(field).unwrap().is_null());
    }
    let toolchain = Toolchain {
        identity,
        rustc: report["compiler"].as_str().unwrap().into(),
        sysroot: report["sysroot"].as_str().unwrap().into(),
        driver: report["driver"].as_str().unwrap().into(),
        bridge: report["macro_bridge"].as_str().unwrap().into(),
        fingerprint: report["fingerprint"].as_str().unwrap().into(),
    };
    assert!(!toolchain.fingerprint.is_empty());
    let text = String::from_utf8(success(&mut doctor()).stdout).unwrap();
    assert!(text.starts_with("Nestrs toolchain is ready\n"));
    assert!(text.contains(&format!("driver fingerprint: {}", toolchain.fingerprint)));

    let target = workspace.0.join("未创建 target");
    for path in [Path::new("未创建 target"), target.as_path()] {
        let output = success(doctor().args(["--json", "--target-dir"]).arg(path));
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        let reported_target = Path::new(report["target_directory"].as_str().unwrap());
        assert!(reported_target.is_absolute());
        // Windows current_dir may return the ordinary spelling of a verbatim
        // input path. Compare the existing parent by filesystem identity.
        assert_eq!(
            reported_target.parent().unwrap().canonicalize().unwrap(),
            workspace.0
        );
        assert_eq!(reported_target.file_name(), target.file_name());
        let cache = toolchain.cache_directory(reported_target);
        assert_eq!(report["cache_directory"], cache.to_str().unwrap());
        assert_eq!(
            report["compiler_output_directory"],
            cache.join("nestrs").join("compiler").to_str().unwrap()
        );
    }

    let failed = doctor()
        .arg("--json")
        .env("NESTRS_DRIVER", workspace.0.join("missing-driver"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert!(failed.stdout.is_empty());
    assert!(String::from_utf8_lossy(&failed.stderr).contains("cannot find compiler driver"));
    assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 0);
}

#[test]
fn native_tools_compile_run_export_and_refresh_ide_in_paths_with_spaces() {
    let workspace = Workspace::new();
    let application = workspace.fixture("ide");
    for operation in ["check", "build", "run"] {
        success(&mut workspace.command(operation, &application));
    }

    // The diagnostic entry must link real registrations without executing any
    // business code, even when replacing a previously exported file on Windows.
    let graph_manifest = workspace.fixture("graph");
    let html_path = workspace.0.join("依赖 graph.html");
    let sentinel = workspace.0.join("business-executed");
    fs::write(&html_path, "old graph output").unwrap();
    for _ in 0..2 {
        success(
            workspace
                .command("graph", &graph_manifest)
                .args(["-p", "nestrs-graph-fixture", "--bin", "alpha", "--output"])
                .arg(&html_path)
                .env("NESTRS_GRAPH_SENTINEL", &sentinel),
        );
        assert!(!sentinel.exists(), "graph executed business code");
        let html = fs::read_to_string(&html_path).unwrap();
        assert!(html.contains("AlphaOnly"));
        assert!(html.contains("Consumer"));
        assert!(!html.contains("old graph output"));
    }
    let valid_graph = fs::read(&html_path).unwrap();
    let invalid_graph = workspace
        .command("graph", &graph_manifest)
        .args([
            "-p",
            "nestrs-graph-fixture",
            "--bin",
            "invalid_graph",
            "--output",
        ])
        .arg(&html_path)
        .env("NESTRS_GRAPH_SENTINEL", &sentinel)
        .output()
        .unwrap();
    assert!(!invalid_graph.status.success());
    assert!(
        String::from_utf8_lossy(&invalid_graph.stderr).contains("[NESTRS-DI001]"),
        "{}",
        String::from_utf8_lossy(&invalid_graph.stderr),
    );
    assert_eq!(fs::read(&html_path).unwrap(), valid_graph);
    assert!(!sentinel.exists(), "invalid graph executed business code");

    // Package reports keep good binaries and report invalid/unsupported entries.
    // Every iteration also replaces the existing file on Windows.
    let mut previous_report = None;
    for _ in 0..2 {
        let result = workspace
            .command("graph", &graph_manifest)
            .args(["-p", "nestrs-graph-fixture", "--output"])
            .arg(&html_path)
            .env("NESTRS_GRAPH_SENTINEL", &sentinel)
            .output()
            .unwrap();
        assert!(
            !result.status.success(),
            "invalid entry must affect exit status"
        );
        let report = graph_data(&html_path);
        assert_eq!(
            report["version"],
            2,
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(report["kind"], "project");
        let entries = report["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 9);
        for binary in ["alpha", "beta"] {
            let entry = entries
                .iter()
                .find(|entry| entry["binary"] == binary)
                .unwrap();
            assert_eq!(entry["status"], "ok", "{entry}");
            assert_eq!(entry["graph"]["nodes"].as_array().unwrap().len(), 5);
        }
        let build_named = entries
            .iter()
            .find(|entry| entry["binary"] == "build-script-build")
            .unwrap();
        assert_eq!(build_named["status"], "ok", "{build_named}");
        assert_eq!(build_named["graph"]["nodes"].as_array().unwrap().len(), 1);
        let macro_main = entries
            .iter()
            .find(|entry| entry["binary"] == "macro_main")
            .unwrap();
        assert_eq!(macro_main["status"], "ok", "{macro_main}");
        assert_eq!(macro_main["graph"]["nodes"].as_array().unwrap().len(), 1);
        let invalid = entries
            .iter()
            .find(|entry| entry["binary"] == "invalid_graph")
            .unwrap();
        assert_eq!(invalid["status"], "error", "{invalid}");
        assert!(invalid["graph"].is_null());
        assert!(!invalid["diagnostic"].as_str().unwrap().is_empty());
        for binary in ["no_main", "cfg_no_main"] {
            let entry = entries
                .iter()
                .find(|entry| entry["binary"] == binary)
                .unwrap();
            assert_eq!(entry["status"], "ok", "{entry}");
            assert_eq!(entry["graph"]["nodes"].as_array().unwrap().len(), 0);
        }
        for binary in ["feature_app", "compile_error"] {
            let entry = entries
                .iter()
                .find(|entry| entry["binary"] == binary)
                .unwrap();
            assert_eq!(entry["status"], "skipped", "{entry}");
        }
        if let Some(previous) = &previous_report {
            assert_eq!(&report, previous, "cached project report must be stable");
        }
        previous_report = Some(report);
        assert!(!sentinel.exists(), "project report executed business code");
    }

    let project_path = workspace.0.join("editor 模型/rust-project.json");
    success(
        workspace
            .command("init", &application)
            .arg("--output")
            .arg(&project_path)
            .arg("--vscode"),
    );
    let project_bytes = fs::read(&project_path).unwrap();
    let project: Value = serde_json::from_slice(&project_bytes).unwrap();
    let crates = project["crates"].as_array().unwrap();
    let bridge = crates
        .iter()
        .find(|item| item["display_name"] == "nestrs")
        .expect("the editor must receive the private declaration bridge");
    let bridge_path = bridge["proc_macro_dylib_path"].as_str().unwrap();
    assert!(Path::new(bridge_path).is_file());
    assert!(bridge_path.ends_with(std::env::consts::DLL_SUFFIX));
    assert!(crates.iter().any(|item| {
        item["env"]["NESTRS_FIXTURE_LABEL"] == "generated-by-build-script"
            && item["cfg"]
                .as_array()
                .unwrap()
                .iter()
                .any(|cfg| cfg == "nestrs_fixture_generated")
    }));

    let settings: Value = serde_json::from_slice(
        &fs::read(application.parent().unwrap().join(".vscode/settings.json")).unwrap(),
    )
    .unwrap();
    let macro_server = settings["rust-analyzer.procMacro.server"].as_str().unwrap();
    assert!(Path::new(macro_server).is_file());
    assert!(macro_server.ends_with(std::env::consts::EXE_SUFFIX));
    let check = settings["rust-analyzer.check.overrideCommand"]
        .as_array()
        .unwrap();
    assert_eq!(check[1], "init");
    assert_eq!(check[2], "check");
    let mut check_command = Command::new(check[0].as_str().unwrap());
    check_command.args(check[1..].iter().map(|arg| arg.as_str().unwrap()));
    clear_build_overrides(&mut check_command);
    for (name, value) in settings["rust-analyzer.check.extraEnv"]
        .as_object()
        .unwrap()
    {
        check_command.env(name, value.as_str().unwrap());
    }
    let checked = success(&mut check_command);
    assert!(
        String::from_utf8_lossy(&checked.stdout)
            .lines()
            .any(|line| {
                serde_json::from_str::<Value>(line).is_ok_and(|message| {
                    message["reason"] == "build-finished" && message["success"] == true
                })
            })
    );
    assert_eq!(fs::read(&project_path).unwrap(), project_bytes);
}

#[test]
fn native_ide_preserves_nonstandard_cargo_source_paths_and_constructor_models() {
    let root = std::env::temp_dir().join(format!("nestrs input paths 空格 {}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let workspace = Workspace(root);
    let manifest = workspace.fixture("ide");
    let project = manifest.parent().unwrap();
    let original_manifest = fs::read_to_string(&manifest).unwrap();
    let source = fs::read_to_string(project.join("src/main.rs")).unwrap();
    fs::remove_file(project.join("src/main.rs")).unwrap();
    for name in ["入口.code", "入口 无后缀", "入口 普通.rs"] {
        let root = project.join("src").join(name);
        fs::write(&root, &source).unwrap();
        fs::write(
            &manifest,
            format!(
                "{original_manifest}\n[[bin]]\nname = \"nestrs-ide-fixture\"\npath = {}\n",
                serde_json::to_string(&format!("src/{name}")).unwrap()
            ),
        )
        .unwrap();
        let model = workspace.0.join("model/rust-project.json");
        success(
            workspace
                .command("init", &manifest)
                .args(["--bin=nestrs-ide-fixture", "--output"])
                .arg(&model)
                .env(
                    "CARGO_ENCODED_RUSTFLAGS",
                    "--remap-path-prefix\x1funused.rs=remapped.rs",
                ),
        );
        let model: Value = serde_json::from_slice(&fs::read(model).unwrap()).unwrap();
        let entry = model["crates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["display_name"] == "nestrs_ide_fixture")
            .expect("Cargo binary must remain an editor crate");
        assert_eq!(
            Path::new(entry["root_module"].as_str().unwrap())
                .canonicalize()
                .unwrap(),
            root.canonicalize().unwrap()
        );
        let constructors: Value =
            serde_json::from_str(entry["env"]["NESTRS_IDE_CONSTRUCTORS"].as_str().unwrap())
                .unwrap();
        let methods = constructors["methods"].as_array().unwrap();
        assert!(!methods.is_empty());
        for method in methods {
            let file = Path::new(method["anchor"]["file"].as_str().unwrap());
            assert!(file.is_absolute(), "constructor method anchor: {method}");
            assert_eq!(file, root.canonicalize().unwrap());
        }
        assert!(
            constructors["declarations"]
                .as_array()
                .unwrap()
                .iter()
                .any(|declaration| declaration["selection"]["constructor"] == true)
        );
        fs::remove_file(root).unwrap();
    }
}
