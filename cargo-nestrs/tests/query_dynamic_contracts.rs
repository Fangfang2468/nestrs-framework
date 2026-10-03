//! Finite ordinary concrete-to-dyn paths preserve query roots across crate metadata.
#![cfg(feature = "compiler-driver")]
use std::{path::Path, process::Command};
fn fixture(
    command: &str,
    release: bool,
    feature: Option<&str>,
    binary: &str,
) -> std::process::Output {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/query-dynamic/Cargo.toml");
    let mut process = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    process
        .args([command, "--offline", "--locked", "--manifest-path"])
        .arg(manifest)
        .args(["--bin", binary]);
    if release {
        process.arg("--release");
    }
    if let Some(feature) = feature {
        process.args(["--features", feature]);
    }
    process
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .expect("run dynamic query-root fixture")
}
#[test]
fn ordinary_dynamic_calls_preserve_queries_without_seeding_unused_methods() {
    for release in [false, true] {
        for feature in [None, Some("extra-root")] {
            let output = fixture("run", release, feature, "nestrs-query-dynamic");
            assert!(
                output.status.success(),
                "release={release} feature={feature:?}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}
#[test]
fn ordinary_dynamic_calls_reject_invalid_graphs_before_execution() {
    for release in [false, true] {
        for command in ["check", "build"] {
            for feature in ["invalid-dynamic", "invalid-default"] {
                let output = fixture(command, release, Some(feature), "nestrs-query-dynamic");
                let stderr = String::from_utf8_lossy(&output.stderr);
                assert!(
                    !output.status.success(),
                    "{command} unexpectedly accepted {feature}"
                );
                assert_eq!(
                    stderr.matches("[NESTRS-DI001]").count(),
                    1,
                    "{command} release={release} feature={feature}\n{stderr}"
                );
                assert!(stderr.contains("Unregistered"), "{stderr}");
                assert!(
                    !stderr.contains("internal compiler error") && !stderr.contains("panicked at"),
                    "{stderr}"
                );
            }
        }
    }
}

#[test]
fn projected_supertrait_parameters_preserve_real_queries_and_invalid_dependencies() {
    for release in [false, true] {
        let output = fixture("run", release, None, "projected_super");
        assert!(
            output.status.success(),
            "projected supertrait release={release}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout)
                .contains("projected supertrait query contracts passed")
        );
        for command in ["check", "build"] {
            let output = fixture(
                command,
                release,
                Some("invalid-projected"),
                "projected_super",
            );
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                !output.status.success(),
                "{command} accepted invalid projected query"
            );
            assert_eq!(
                stderr.matches("[NESTRS-DI001]").count(),
                1,
                "{command} release={release}\n{stderr}"
            );
            assert!(
                stderr.contains("InvalidRepository")
                    && stderr.contains("Unregistered")
                    && stderr.contains("InvalidTag")
                    && !stderr.contains("RejectedTag"),
                "{stderr}"
            );
            assert!(
                !stderr.contains("internal compiler error") && !stderr.contains("panicked at"),
                "{stderr}"
            );
        }
    }
}
