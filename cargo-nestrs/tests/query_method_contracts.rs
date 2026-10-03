//! 查询根来自编译器类型语义，Debug 与 Release 的未执行分支应贡献相同图。
//! fixture 同时断言上游泛型 Iterator/Add、运算符、trait/inherent 方法和关联常量
//! 函数指针的 Eager 构造计数与真实查询结果，防止调用或常量身份被提前过滤。
#![cfg(feature = "compiler-driver")]

use std::{path::Path, process::Command};

#[test]
fn query_methods_collect_closed_roots_across_helpers_and_crates() {
    let package = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = package.join("tests/fixtures/query-methods/Cargo.toml");
    for (release, extra) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .args(["run", "--offline", "--manifest-path"])
            .arg(&manifest);
        if release {
            command.arg("--release");
        }
        if extra {
            command.args(["--features", "extra-root"]);
        }
        let output = command
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .output()
            .expect("run compiler query-root fixture");
        assert!(
            output.status.success(),
            "query methods release={release} extra-root={extra}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for release in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .args(["run", "--offline", "--manifest-path"])
            .arg(&manifest)
            .args(["-p", "indirect-app"]);
        if release {
            command.arg("--release");
        }
        let output = command
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "transitive app release={release}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
