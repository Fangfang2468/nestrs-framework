//! The CLI uses real rustdoc semantics and the driver for executable doctests.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct TargetDirectory(PathBuf);

impl TargetDirectory {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nestrs-rustdoc-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TargetDirectory {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run_examples(doc_only: bool) {
    let target = TargetDirectory::new();
    let records = target.0.join("executed-examples");
    fs::create_dir(&records).unwrap();
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/macro-rustdoc/Cargo.toml");
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    command.arg("test");
    if doc_only {
        command.arg("--doc");
    }
    command
        .args(["--locked", "--offline", "--manifest-path"])
        .arg(&manifest)
        .args(["--", "--nocapture"])
        .env("CARGO_TARGET_DIR", &target.0)
        .env("NESTRS_DOCTEST_RECORD", &records)
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTDOC")
        .env_remove("NESTRS_REAL_RUSTDOC")
        .env_remove("NESTRS_GRAPH_TARGET");
    command.env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"));
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains("12 passed; 0 failed; 1 ignored"),
        "{stdout}\n{stderr}"
    );
    for marker in [
        "library",
        "declarations",
        "generics",
        "method",
        "trait-method",
        "complex-impl",
        "included",
        "doc-cfg",
    ] {
        assert_eq!(
            fs::read_to_string(records.join(marker)).unwrap(),
            "passed",
            "example {marker} did not finish its runtime assertions",
        );
    }
}

#[test]
fn cargo_nestrs_test_includes_executable_documentation_examples() {
    run_examples(false);
}

#[test]
fn cargo_nestrs_runs_macro_documentation_examples() {
    run_examples(true);
}

#[test]
fn cargo_nestrs_doctests_preserve_custom_root_paths_and_ignore_source_like_option_values() {
    let target = TargetDirectory::new();
    let project = target.0.join("project 空格");
    fs::create_dir_all(project.join("源码 目录")).unwrap();
    let manifest = project.join("Cargo.toml");
    // 这是合法的 remap 选项值，同时故意创建同名 .rs 文件。不能因其存在而把它
    // 当成输入，也不能用字符串相等替换 rustdoc 的选项值。
    fs::write(
        project.join("unused.rs=remapped.rs"),
        "compile_error!(\"not the crate root\");",
    )
    .unwrap();
    fs::write(
        project.join("源码 目录/relative-example.rs"),
        "{ assert_eq!(custom_roots::answer(), 42); }",
    )
    .unwrap();
    for name in ["库.code", "库 无后缀", "库 普通.rs"] {
        let source = project.join("源码 目录").join(name);
        fs::write(&source, "//! ```\n//! include!(\"relative-example.rs\");\n//! ```\npub fn answer() -> u8 { 42 }\n").unwrap();
        fs::write(&manifest, format!(
            "[package]\nname = \"custom-roots\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[lib]\npath = {}\n",
            serde_json::to_string(&format!("源码 目录/{name}")).unwrap(),
        )).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
            .args(["test", "--doc", "--offline", "--manifest-path"])
            .arg(&manifest)
            .arg("--target-dir")
            .arg(target.0.join("cargo"))
            .current_dir(&project)
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env(
                "CARGO_ENCODED_RUSTDOCFLAGS",
                "--remap-path-prefix\x1funused.rs=remapped.rs",
            )
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTDOCFLAGS")
            .env_remove("RUSTDOC")
            .env_remove("NESTRS_REAL_RUSTDOC")
            .env_remove("NESTRS_IDE_CAPTURE")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{name}: {stdout}\n{stderr}");
        assert!(
            stdout.contains("1 passed; 0 failed"),
            "{name}: {stdout}\n{stderr}"
        );
        fs::remove_file(source).unwrap();
    }
}
