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
