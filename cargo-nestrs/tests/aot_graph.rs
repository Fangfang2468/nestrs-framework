//! 非法图在最终入口的编译阶段失败，测试程序不启动任何用户入口或服务构造。
#![cfg(feature = "compiler-driver")]
use std::{path::Path, process::Command};

#[test]
fn invalid_graphs_are_rejected_by_check_and_build() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("tests/fixtures/aot-invalid/Cargo.toml");
    for (binary, kind, details) in [
        (
            "missing",
            "MissingDependency",
            &["UnusedBroken", "Missing", "missing"][..],
        ),
        ("cycle", "Cycle", &["Alpha", "Beta", "beta", "alpha"][..]),
        (
            "scope",
            "ScopeRequired",
            &["ApplicationCache", "RequestSession"][..],
        ),
        ("duplicate", "DuplicateProvider", &["Service"][..]),
        (
            "ambiguous",
            "AmbiguousTrait",
            &["Port", "First", "Second"][..],
        ),
        ("primary", "AmbiguousTrait", &["primary"][..]),
        (
            "lazy_scope",
            "ScopeRequired",
            &["Application", "Intermediate", "Session"][..],
        ),
        ("lazy_cycle", "Cycle", &["First", "Second"][..]),
        (
            "lazy_provider_missing",
            "MissingDependency",
            &["DeferredUnused", "Missing"][..],
        ),
        (
            "lazy_provider_cycle",
            "Cycle",
            &["DeferredAlpha", "DeferredBeta"][..],
        ),
        (
            "lazy_provider_scope",
            "ScopeRequired",
            &["DeferredApplication", "DeferredSession"][..],
        ),
        (
            "lazy_factory_missing",
            "MissingDependency",
            &["LazyFactory", "Missing", "missing"][..],
        ),
        ("lazy_factory_cycle", "Cycle", &["First", "Second"][..]),
        (
            "lazy_factory_scope",
            "ScopeRequired",
            &["Application", "Intermediate", "Session"][..],
        ),
        (
            "generic_growth",
            "DI 泛型类型不断增长或过于复杂",
            &["1024", "递归泛型查询"][..],
        ),
        (
            "associated_const_growth",
            "DI 泛型类型不断增长或过于复杂",
            &["1024", "递归泛型查询"][..],
        ),
    ] {
        for operation in ["check", "build"] {
            let output = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
                .args([operation, "--offline", "--manifest-path"])
                .arg(&manifest)
                .args(["--bin", binary])
                .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
                .env_remove("RUSTC_BOOTSTRAP")
                .output()
                .expect("compiler should start");
            let diagnostic = String::from_utf8_lossy(&output.stderr);
            assert!(
                !output.status.success(),
                "{operation} {binary} accepted an invalid graph"
            );
            assert!(
                diagnostic.contains(kind),
                "{operation} {binary}: {diagnostic}"
            );
            for detail in details {
                assert!(
                    diagnostic.contains(detail),
                    "missing {detail}: {diagnostic}"
                );
            }
            assert!(
                !diagnostic.contains("internal compiler error"),
                "{diagnostic}"
            );
            assert!(
                !diagnostic.contains("must not execute"),
                "a constructor executed during compilation: {diagnostic}"
            );
        }
    }
}
