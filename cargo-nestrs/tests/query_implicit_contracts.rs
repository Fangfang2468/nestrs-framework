//! 隐式 Deref/DerefMut 与原生析构边使用跨 crate 的真实类型和 drop glue。
//! 每条路径有独立服务计数；抑制析构的类型故意带非法依赖，防止粗筛制造假根。
#![cfg(feature = "compiler-driver")]

use std::{path::Path, process::Command};

fn fixture(command: &str, release: bool, feature: Option<&str>) -> std::process::Output {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/query-implicit/Cargo.toml");
    let mut process = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    process
        .args([command, "--offline", "--manifest-path"])
        .arg(manifest);
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
        .expect("run implicit query-root fixture")
}

#[test]
fn implicit_calls_preserve_closed_queries_and_native_drop_semantics() {
    for release in [false, true] {
        for feature in [None, Some("extra-root")] {
            let output = fixture("run", release, feature);
            assert!(
                output.status.success(),
                "implicit roots release={release} feature={feature:?}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn implicit_queries_reject_invalid_graphs_during_check_and_build() {
    for release in [false, true] {
        for command in ["check", "build"] {
            for feature in ["invalid-deref", "invalid-drop", "invalid-associated"] {
                let output = fixture(command, release, Some(feature));
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
            }
        }
    }
}
