//! Closed calls through ordinary upstream libraries retain query roots even when
//! the upstream library has no core dependency and no Nestrs query metadata.
#![cfg(feature = "compiler-driver")]

use std::{path::Path, process::Command};

fn fixture(command: &str, release: bool, features: &str) -> std::process::Output {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/query-external/Cargo.toml");
    let mut process = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    process
        .args([command, "--offline", "--locked", "--manifest-path"])
        .arg(manifest);
    if release {
        process.arg("--release");
    }
    if !features.is_empty() {
        process.args(["--features", features]);
    }
    process
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .expect("run ordinary external query-root fixture")
}

#[test]
fn ordinary_external_calls_preserve_queries_without_a_core_dependency() {
    for release in [false, true] {
        // The core-control feature only changes the helper manifests. The library
        // and application source are identical; direct-only is an isolated control.
        for features in [
            "",
            "helper-core",
            "direct-only",
            "direct-only,primitive-direct",
        ] {
            let output = fixture("run", release, features);
            assert!(
                output.status.success(),
                "release={release} features={features}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                String::from_utf8_lossy(&output.stdout).contains("external query contracts passed")
            );
        }
    }
}

#[test]
fn ordinary_external_queries_validate_graphs_before_execution() {
    for release in [false, true] {
        for command in ["check", "build"] {
            for path in [
                "forward",
                "primitive",
                "family",
                "family-default",
                "family-return",
                "family-nested",
                "family-dynamic",
                "erasure",
                "deep",
                "closure",
                "default",
                "associated",
                "unexecuted",
                "unexecuted-erasure",
                "unexecuted-associated",
                "direct",
            ] {
                // Exclude all other routes so no independently closed query can
                // seed the service or mask a lost external call/constant edge.
                let features = format!("direct-only,invalid-{path}");
                assert_invalid(command, release, &features);
            }
            // Strict same-source controls for the reported forward and erasure
            // routes and the branch eliminated by native MIR optimization.
            for path in [
                "forward",
                "erasure",
                "unexecuted",
                "family",
                "family-default",
                "family-nested",
            ] {
                let features = format!("direct-only,helper-core,invalid-{path}");
                assert_invalid(command, release, &features);
            }
        }
    }
}

fn assert_invalid(command: &str, release: bool, features: &str) {
    let output = fixture(command, release, features);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "{command} release={release} accepted {features}"
    );
    assert_eq!(
        stderr.matches("[NESTRS-DI001]").count(),
        1,
        "{command} release={release} features={features}\n{stderr}"
    );
    assert!(
        stderr.contains("InvalidRepository")
            && stderr.contains("BadTag")
            && stderr.contains("Unregistered"),
        "{stderr}"
    );
    assert!(
        !stderr.contains("internal compiler error") && !stderr.contains("panicked at"),
        "{stderr}"
    );
}
