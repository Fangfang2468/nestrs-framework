//! Internal registration/activation contracts compiled inside the real core.
//!
//! Each case gets a separate crate identity and registry. Test code uses normal
//! same-crate privacy; application compilation never receives extra access.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const CASES: &[&str] = &[
    "registration_collection",
    "injectable_provider",
    "factory_provider",
    "generic_provider",
    "open_generic_provider",
    "bound_provider",
    "registered_roots",
    "implicit_drop_roots",
    "runtime_bound_generic",
    "runtime_field_cfg",
    "runtime_graph_display",
    "runtime_graph_initialization",
];

/// Preserve real source/test paths so private modules and their white-box tests
/// resolve exactly as they do in core, including crate-root inner documentation.
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let target = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn project(root: &Path, directory: &Path, case: &str) -> PathBuf {
    fs::create_dir_all(directory).unwrap();
    let manifest = directory.join("Cargo.toml");
    fs::write(
        &manifest,
        format!(
            r#"[package]
name = "nestrs-core-contract-{case}"
version = "0.0.0"
edition = "2024"
publish = false

[workspace]

[lib]
name = "nestrs_core"
path = "src/lib.rs"
doctest = false

[dependencies]
ahash = "0.8.12"
thiserror = "2"
serde_json = "1"
tokio = {{ version = "1.53.1", default-features = false, features = ["rt-multi-thread", "sync", "macros", "time"] }}

[lints.rust]
unexpected_cfgs = {{ level = "warn", check-cfg = ['cfg(nestrs_compiler)', 'cfg(nestrs_compiler_contract)'] }}
"#,
        ),
    )
    .unwrap();
    let core = root.join("nestrs-core");
    for name in ["src", "tests"] {
        copy_tree(&core.join(name), &directory.join(name));
    }
    let crate_root = directory.join("src/lib.rs");
    let source = fs::read_to_string(&crate_root).unwrap();
    fs::write(
        crate_root,
        format!(
            "#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]\n\
             {source}\n\
             extern crate self as nestrs_core;\n\
             #[cfg(test)]\n\
             #[path = \"../tests/compiler/{case}.rs\"]\n\
             mod contract;\n",
        ),
    )
    .unwrap();
    manifest
}

#[test]
fn generated_declarations_obey_internal_core_contracts_without_public_bridges() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let artifacts = workspace
        .join("target/compiler-contracts")
        .join(std::process::id().to_string());
    let mut results = Vec::new();
    for case in CASES {
        let manifest = project(workspace, &artifacts.join(case), case);
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
            .args(["test", "--offline", "--lib", "--manifest-path"])
            .arg(&manifest)
            .args(["--", "contract::", "--test-threads=1"])
            .env("CARGO_TARGET_DIR", artifacts.join("build"))
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env("RUSTFLAGS", "--cfg nestrs_compiler_contract")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .output()
            .unwrap();
        let log = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        fs::write(artifacts.join(format!("{case}.log")), &log).unwrap();
        results.push(serde_json::json!({ "case": case, "passed": output.status.success() }));
        fs::write(
            artifacts.join("results.json"),
            serde_json::to_vec_pretty(&results).unwrap(),
        )
        .unwrap();
        assert!(output.status.success(), "{case}: {log}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("test contract::") && stdout.contains("test result: ok."),
            "{case}: the real test executable did not report success: {log}",
        );
    }
}
