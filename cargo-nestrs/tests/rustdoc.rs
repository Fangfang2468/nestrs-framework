//! The CLI supplies its private bridge to real rustdoc and executable doctests.
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
        stdout.contains("3 passed; 0 failed; 0 ignored"),
        "{stdout}\n{stderr}"
    );
    for marker in ["library", "declarations", "generics"] {
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
