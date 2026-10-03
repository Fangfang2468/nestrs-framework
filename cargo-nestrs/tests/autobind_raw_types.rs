//! Source printing must preserve raw associated names and their typed bindings.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn raw_associated_types_survive_automatic_binding_source_generation() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = workspace.join(format!(
        "target/autobind-raw-types/{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    // Keep the existing locked regression dependency set; this test owns only
    // a generated project below target and never edits the checked-in fixture.
    fs::write(
        directory.join("Cargo.toml"),
        format!(
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
"#,
            serde_json::to_string(&workspace.join("nestrs-core")).unwrap(),
        ),
    )
    .unwrap();
    let lock = fs::read(workspace.join("cargo-nestrs/tests/fixtures/di/Cargo.lock")).unwrap();
    fs::write(directory.join("Cargo.lock"), &lock).unwrap();
    let source = include_bytes!("fixtures/raw-types/main.rs");
    fs::write(directory.join("src/main.rs"), source).unwrap();

    for release in [false, true] {
        for operation in ["check", "run"] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
            command
                .args([operation, "--locked", "--offline"])
                .current_dir(&directory)
                .env("CARGO_TARGET_DIR", directory.join("build"))
                .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
                .env_remove("RUSTC_BOOTSTRAP")
                .env_remove("RUSTC_WRAPPER")
                .env_remove("RUSTC_WORKSPACE_WRAPPER")
                .env_remove("RUSTFLAGS")
                .env_remove("CARGO_ENCODED_RUSTFLAGS");
            if release {
                command.arg("--release");
            }
            let output = command.output().unwrap();
            let log = format!(
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            let profile = if release { "release" } else { "debug" };
            fs::write(directory.join(format!("{profile}-{operation}.log")), &log).unwrap();
            assert!(output.status.success(), "{profile} {operation}\n{log}");
            if operation == "run" {
                assert!(log.contains("raw associated type source paths passed"));
            }
        }
    }
    assert_eq!(fs::read(directory.join("Cargo.lock")).unwrap(), lock);
    assert_eq!(fs::read(directory.join("src/main.rs")).unwrap(), source);
}
