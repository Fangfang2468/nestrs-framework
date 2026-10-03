//! 上游 rlib 中的预热策略须经 metadata 保留，泛型在下游闭合后沿用蓝图策略。
#![cfg(feature = "compiler-driver")]
use std::{fs, path::Path, process::Command};

#[test]
fn upstream_provider_lazy_policies_survive_debug_and_release_metadata() {
    let manifest =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/provider-lazy/Cargo.toml");
    for release in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .args(["run", "--offline", "--manifest-path"])
            .arg(&manifest);
        if release {
            command.arg("--release");
        }
        let result = command
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .output()
            .expect("run cross-crate provider policy fixture");
        assert!(
            result.status.success(),
            "release={release}\n{}\n{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("cross-crate provider lazy:"));
    }
}

#[test]
fn upstream_factory_lazy_parameter_is_preserved_in_graph_metadata() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = root.join("tests/fixtures/provider-lazy/Cargo.toml");
    let graph = root.join("../target/factory-lazy-cross-crate.html");
    let result = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
        .args(["graph", "--offline", "--manifest-path"])
        .arg(&manifest)
        .args(["--bin", "nestrs-provider-lazy-app", "--output"])
        .arg(&graph)
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .expect("export upstream factory lazy graph");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let html = fs::read_to_string(graph).unwrap();
    let script = html.split_once("id=\"graph-data\"").unwrap().1;
    let json = script
        .split_once('>')
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let graph: serde_json::Value = serde_json::from_str(json).unwrap();
    let nodes = graph["nodes"].as_array().unwrap();
    let consumer = nodes
        .iter()
        .find(|node| node["label"] == "DeferredConsumer")
        .unwrap();
    let input = &consumer["dependencies"][0];
    assert_eq!(input["lazy"], true);
    assert_eq!(input["label"], "target");
    assert!(
        input["target"].is_number(),
        "延迟参数也必须在编译期选择具体服务"
    );
    let target = nodes
        .iter()
        .find(|node| node["id"] == input["target"])
        .unwrap();
    assert_eq!(target["label"], "ParameterTarget");
}
