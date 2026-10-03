//! Ordinary Rust types and passive query carriers do not consume DI budgets.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const CASES: &[(&str, &str)] = &[
    ("ordinary", include_str!("fixtures/type-budget/ordinary.rs")),
    ("provider", include_str!("fixtures/type-budget/provider.rs")),
    (
        "projected_provider",
        include_str!("fixtures/type-budget/projected_provider.rs"),
    ),
    ("factory", include_str!("fixtures/type-budget/factory.rs")),
    ("query", include_str!("fixtures/type-budget/query.rs")),
    (
        "generic_growth",
        include_str!("fixtures/aot-invalid/src/bin/generic_growth.rs"),
    ),
    (
        "associated_const_growth",
        include_str!("fixtures/aot-invalid/src/bin/associated_const_growth.rs"),
    ),
    (
        "carrier_direct",
        include_str!("fixtures/type-budget/carrier_direct.rs"),
    ),
    (
        "carrier_erased",
        include_str!("fixtures/type-budget/carrier_erased.rs"),
    ),
    (
        "carrier_independent",
        include_str!("fixtures/type-budget/carrier_independent.rs"),
    ),
    (
        "carrier_query_direct",
        include_str!("fixtures/type-budget/carrier_query_direct.rs"),
    ),
    (
        "carrier_query_erased",
        include_str!("fixtures/type-budget/carrier_query_erased.rs"),
    ),
    (
        "carrier_growth",
        include_str!("fixtures/type-budget/carrier_growth.rs"),
    ),
    (
        "trait_empty_impl",
        include_str!("fixtures/type-budget/trait_empty_impl.rs"),
    ),
    (
        "trait_split_control",
        include_str!("fixtures/type-budget/trait_split_control.rs"),
    ),
    (
        "trait_generic_helper",
        include_str!("fixtures/type-budget/trait_generic_helper.rs"),
    ),
    (
        "trait_default_forward",
        include_str!("fixtures/type-budget/trait_default_forward.rs"),
    ),
    (
        "trait_default_override",
        include_str!("fixtures/type-budget/trait_default_override.rs"),
    ),
    (
        "trait_associated_const",
        include_str!("fixtures/type-budget/trait_associated_const.rs"),
    ),
    (
        "fixed_query_generic",
        include_str!("fixtures/type-budget/fixed_query_generic.rs"),
    ),
    (
        "query_projected",
        include_str!("fixtures/type-budget/query_projected.rs"),
    ),
    (
        "projected_small_control",
        include_str!("fixtures/type-budget/projected_small_control.rs"),
    ),
    (
        "trait_distinct_impls",
        include_str!("fixtures/type-budget/trait_distinct_impls.rs"),
    ),
    (
        "trait_distinct_split_control",
        include_str!("fixtures/type-budget/trait_distinct_split_control.rs"),
    ),
    (
        "trait_distinct_default",
        include_str!("fixtures/type-budget/trait_distinct_default.rs"),
    ),
    (
        "trait_distinct_constants",
        include_str!("fixtures/type-budget/trait_distinct_constants.rs"),
    ),
    (
        "trait_distinct_cross_crate",
        include_str!("fixtures/type-budget/trait_distinct_cross_crate.rs"),
    ),
    (
        "trait_distinct_cross_crate_control",
        include_str!("fixtures/type-budget/trait_distinct_cross_crate_control.rs"),
    ),
    (
        "trait_method_growth",
        include_str!("fixtures/type-budget/trait_method_growth.rs"),
    ),
    (
        "trait_constant_growth",
        include_str!("fixtures/type-budget/trait_constant_growth.rs"),
    ),
];

const DISTINCT_LIBRARY: &str = include_str!("fixtures/type-budget/distinct_library.rs");

#[test]
fn ordinary_complex_types_do_not_consume_di_budgets_and_real_di_growth_is_rejected() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = workspace.join(format!(
        "target/type-budget-contracts/{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src/bin")).unwrap();
    let manifest = format!(
        r#"[package]
name = "nestrs-di-regressions"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
nestrs-core = {{ path = {} }}
tokio = {{ version = "1.53.1", default-features = false, features = ["rt-multi-thread", "sync", "macros", "time"] }}
serde_json = "1"
[features]
with-di = []
"#,
        serde_json::to_string(&workspace.join("nestrs-core")).unwrap(),
    );
    fs::write(directory.join("Cargo.toml"), &manifest).unwrap();
    let lock = fs::read(workspace.join("cargo-nestrs/tests/fixtures/di/Cargo.lock")).unwrap();
    fs::write(directory.join("Cargo.lock"), &lock).unwrap();
    // Distinct array lengths create distinct rustc types; repeating the same
    // tuple element would be deduplicated by Ty::walk and miss this boundary.
    let wide = format!(
        "type Wide = ({});\n",
        (0..1100)
            .map(|length| format!("[u8; {length}],"))
            .collect::<String>()
    );
    fs::write(directory.join("src/wide.rs"), &wide).unwrap();
    let wide_generic = wide.replace("type Wide = (", "type WideOf<T> = (T,");
    fs::write(directory.join("src/wide_generic.rs"), &wide_generic).unwrap();
    fs::write(directory.join("src/lib.rs"), DISTINCT_LIBRARY).unwrap();
    for &(name, source) in CASES {
        fs::write(directory.join(format!("src/bin/{name}.rs")), source).unwrap();
    }

    let mut results = Vec::new();
    for release in [false, true] {
        let profile = if release { "release" } else { "debug" };
        let runs = [
            ("ordinary", true, false),
            ("ordinary", false, false),
            ("ordinary", false, true),
        ]
        .into_iter()
        .chain(CASES[1..].iter().map(|(name, _)| (*name, false, false)));
        for (name, native, with_di) in runs {
            let expected_success = matches!(
                name,
                "ordinary"
                    | "carrier_direct"
                    | "carrier_erased"
                    | "carrier_independent"
                    | "trait_empty_impl"
                    | "trait_split_control"
                    | "trait_generic_helper"
                    | "trait_default_forward"
                    | "trait_default_override"
                    | "trait_associated_const"
                    | "fixed_query_generic"
                    | "projected_small_control"
                    | "trait_distinct_impls"
                    | "trait_distinct_split_control"
                    | "trait_distinct_default"
                    | "trait_distinct_constants"
                    | "trait_distinct_cross_crate"
                    | "trait_distinct_cross_crate_control"
            );
            let mut command = Command::new(if native {
                "cargo"
            } else {
                env!("CARGO_BIN_EXE_cargo-nestrs")
            });
            command
                .args([
                    if expected_success { "run" } else { "check" },
                    "--locked",
                    "--offline",
                    "--message-format=json",
                    "--bin",
                    name,
                ])
                .current_dir(&directory)
                .env("CARGO_TARGET_DIR", directory.join("build"))
                .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
                .env("CARGO_TERM_COLOR", "never");
            for key in [
                "RUSTC_BOOTSTRAP",
                "RUSTC_WRAPPER",
                "RUSTC_WORKSPACE_WRAPPER",
                "RUSTFLAGS",
                "CARGO_ENCODED_RUSTFLAGS",
                "NESTRS_GRAPH_TARGET",
                "NESTRS_IDE_CAPTURE",
            ] {
                command.env_remove(key);
            }
            if release {
                command.arg("--release");
            }
            if with_di {
                command.args(["--features", "with-di"]);
            }
            let output = command.output().unwrap();
            let log = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let label = format!(
                "{profile}-{name}-{}",
                if native {
                    "cargo"
                } else if with_di {
                    "nestrs-with-di"
                } else {
                    "nestrs"
                }
            );
            fs::write(directory.join(format!("{label}.log")), &log).unwrap();
            assert_eq!(output.status.success(), expected_success, "{label}\n{log}");
            for forbidden in [
                "internal compiler error",
                "panicked at",
                "fatal runtime error",
            ] {
                assert!(!log.contains(forbidden), "{label}: {forbidden}\n{log}");
            }
            let errors: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
                .filter(|event| {
                    event["reason"] == "compiler-message" && event["message"]["level"] == "error"
                })
                .map(|event| event["message"].clone())
                .collect();
            if expected_success {
                assert!(errors.is_empty(), "{label}\n{log}");
            } else {
                assert_eq!(errors.len(), 1, "{label}\n{log}");
                assert!(
                    errors[0]["message"]
                        .as_str()
                        .unwrap()
                        .starts_with("[NESTRS-DI008]"),
                    "{label}\n{log}"
                );
                assert!(log.contains("TypeExpansionLimit"), "{label}\n{log}");
                assert!(log.contains("single_type_tree_nodes"), "{label}\n{log}");
            }
            results.push(serde_json::json!({ "case": label, "passed": true }));
            fs::write(
                directory.join("results.json"),
                serde_json::to_vec_pretty(&results).unwrap(),
            )
            .unwrap();
        }
    }
    assert_eq!(
        fs::read_to_string(directory.join("Cargo.toml")).unwrap(),
        manifest
    );
    assert_eq!(fs::read(directory.join("Cargo.lock")).unwrap(), lock);
    assert_eq!(
        fs::read_to_string(directory.join("src/wide.rs")).unwrap(),
        wide
    );
    assert_eq!(
        fs::read_to_string(directory.join("src/wide_generic.rs")).unwrap(),
        wide_generic
    );
    assert_eq!(
        fs::read_to_string(directory.join("src/lib.rs")).unwrap(),
        DISTINCT_LIBRARY
    );
    for &(name, source) in CASES {
        assert_eq!(
            fs::read_to_string(directory.join(format!("src/bin/{name}.rs"))).unwrap(),
            source
        );
    }
}
