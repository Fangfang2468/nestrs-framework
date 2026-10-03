//! Compiler-generated Clone shims must retain the closed calls to field Clone.
#![cfg(feature = "compiler-driver")]

use std::{path::Path, process::Command};

const QUERY_PATHS: [&str; 10] = [
    "tuple",
    "nested",
    "closure",
    "external_tuple",
    "external_nested",
    "external_closure",
    "unexecuted",
    "external_unexecuted",
    "direct",
    "array",
];

fn fixture(command: &str, release: bool, binary: &str, invalid: bool) -> std::process::Output {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/query-clone/Cargo.toml");
    let mut process = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    process
        .args([command, "--offline", "--locked", "--manifest-path"])
        .arg(manifest)
        .args(["--bin", binary]);
    if release {
        process.arg("--release");
    }
    if invalid {
        process.args(["--features", "invalid"]);
    }
    process
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .expect("run compiler-generated Clone query-root fixture")
}

#[test]
fn compiler_generated_clone_calls_retain_queries() {
    for release in [false, true] {
        for binary in QUERY_PATHS.into_iter().chain(["multiple", "no_clone"]) {
            let output = fixture("run", release, binary, false);
            assert!(
                output.status.success(),
                "{binary} release={release}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("clone query contracts passed")
            );
        }
    }
}

#[test]
fn compiler_generated_clone_queries_validate_graphs_before_execution() {
    for release in [false, true] {
        for command in ["check", "build"] {
            // Every binary is an isolated route. A direct control cannot seed a
            // closed Repository type for a missing tuple or closure shim edge.
            for binary in QUERY_PATHS {
                let output = fixture(command, release, binary, true);
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    !output.status.success(),
                    "{command} release={release} accepted {binary} with a missing dependency"
                );
                assert_eq!(
                    stderr.matches("[NESTRS-DI001]").count(),
                    1,
                    "{command} release={release} {binary}\n{stderr}"
                );
                assert_eq!(stderr.matches("[NESTRS-DI").count(), 1, "{stderr}");
                assert!(
                    stderr.contains("Repository<u64>") && stderr.contains("Missing"),
                    "{stderr}"
                );
                assert!(
                    !stderr.contains("internal compiler error") && !stderr.contains("panicked at"),
                    "{stderr}"
                );
            }
            // Merely storing Runner in a tuple must not invent a Clone call or
            // materialize its otherwise-invalid generic Repository declaration.
            let output = fixture(command, release, "no_clone", true);
            assert!(
                output.status.success(),
                "{command} release={release} no_clone\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
