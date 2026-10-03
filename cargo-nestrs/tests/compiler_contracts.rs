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
    "runtime_bound_generic",
    "runtime_field_cfg",
    "runtime_graph_display",
    "runtime_graph_initialization",
];

fn quoted(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
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
path = "lib.rs"
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
    fs::write(
        directory.join("lib.rs"),
        format!(
            "#![allow(macro_expanded_macro_exports_accessed_by_absolute_paths)]\n\
             extern crate self as nestrs_core;\n\
             include!({});\n\
             #[cfg(test)]\n\
             #[path = {}]\n\
             mod contract;\n",
            quoted(&root.join("nestrs-core/src/lib.rs")),
            quoted(
                &root
                    .join("nestrs-core/tests/compiler")
                    .join(format!("{case}.rs"))
            ),
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
