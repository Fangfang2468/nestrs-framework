//! 无直接 core 依赖的下游 crate 仍需加载上游宏 metadata，并在 binary/doctest 的最终
//! 入口生成 registry。fixture 实际调用上游封装的 DI，避免链接裁剪掩盖缺失入口。
#![cfg(feature = "compiler-driver")]

use std::{fs, path::Path, process::Command};

#[test]
fn recursive_bridge_metadata_survives_check_build_and_rustdoc_without_direct_core_dependency() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("tests/fixtures/bridge-metadata/Cargo.toml");
    let target =
        std::env::temp_dir().join(format!("nestrs-bridge-metadata-{}", std::process::id()));
    fs::create_dir_all(&target).unwrap();
    for (subcommand, extra, expected) in [
        ("check", vec!["--all-targets"], None),
        (
            "run",
            vec![],
            Some("downstream metadata without a core dependency passed"),
        ),
        ("test", vec!["--doc"], Some("1 passed; 0 failed")),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
            .arg(subcommand)
            .args(extra)
            .args(["--locked", "--offline", "--manifest-path"])
            .arg(&manifest)
            .env("CARGO_TARGET_DIR", &target)
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{subcommand}: {stdout}\n{stderr}");
        if let Some(expected) = expected {
            assert!(
                stdout.contains(expected),
                "{subcommand}: {stdout}\n{stderr}"
            );
        }
    }
    fs::remove_dir_all(target).unwrap();
}
