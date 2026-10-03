//! 生成反射层是最终入口的真实执行清单，不能退化为搬迁后的注册候选目录。
#![cfg(feature = "compiler-driver")]

use std::{fs, path::Path, process::Command};

#[test]
fn generated_manifest_records_selected_inputs_and_never_evaluates_business_initializers() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let directory = workspace
        .join("target/reflection-contracts")
        .join(std::process::id().to_string());
    fs::create_dir_all(directory.join("src")).unwrap();
    let core = serde_json::to_string(&workspace.join("nestrs-core").to_string_lossy()).unwrap();
    fs::write(directory.join("Cargo.toml"), format!(
        "[package]\nname = \"reflection-contract\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n[dependencies]\nnestrs-core = {{ path = {core} }}\n"
    )).unwrap();
    fs::write(
        directory.join("src/main.rs"),
        r#"
#![allow(dead_code)]
use nestrs::{injectable, primary, factory};
trait Store: Send + Sync {}
trait Audit: Send + Sync {}
#[primary]
#[injectable(key = "main")]
struct ZPreferred;
impl Store for ZPreferred {}
#[injectable(key = "main")]
struct ASecondary;
impl Store for ASecondary {}
struct Reports;
#[factory]
fn reports() -> Reports { panic!("must not execute factory") }
#[injectable]
struct Unused {
    #[value({ panic!("must not execute value"); 0usize })]
    value: usize,
}
#[injectable]
struct Checkout {
    #[inject("main")]
    store: dyn Store,
    #[inject]
    #[lazy]
    reports: Reports,
    #[inject]
    audit: Option<dyn Audit>,
}
fn main() { panic!("must not execute main") }
"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
        .args(["check", "--offline"])
        .current_dir(&directory)
        .env("CARGO_TARGET_DIR", directory.join("build"))
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut pending = vec![directory.join("build")];
    let mut manifests = vec![];
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry
                .file_name()
                .to_string_lossy()
                .ends_with(".nestrs-reflect.json")
            {
                manifests.push(entry.path());
            }
        }
    }
    let plans: Vec<serde_json::Value> = manifests
        .iter()
        .map(|path| serde_json::from_slice(&fs::read(path).unwrap()).unwrap())
        .collect();
    let plans: Vec<_> = plans
        .iter()
        .filter(|plan| plan["crate"] == "reflection_contract")
        .collect();
    assert_eq!(plans.len(), 1, "one immutable manifest per final entry");
    let plan = plans[0];
    assert_eq!(plan["format"], "nestrs-reflect");
    assert_eq!(plan["entry"], "__nestrs_reflect_v2");
    let nodes = plan["nodes"].as_array().unwrap();
    assert_eq!(
        nodes.len(),
        5,
        "unused declarations still belong to the validated plan"
    );
    let find = |name: &str| {
        nodes
            .iter()
            .find(|node| node["type"].as_str().unwrap().ends_with(name))
            .unwrap()
    };
    let checkout = find("::Checkout");
    let preferred = find("::ZPreferred");
    let reports = find("::Reports");
    let inputs = checkout["inputs"].as_array().unwrap();
    assert_eq!(inputs.len(), 3);
    assert_eq!(
        plan["projections"].as_array().unwrap().len(),
        1,
        "未被任何输入或查询路由选中的候选投影不应交付 core"
    );
    assert_eq!(inputs[0]["target"], preferred["id"]);
    assert_eq!(inputs[0]["projection"], 0);
    assert!(
        plan["projections"][0]["concrete"]
            .as_str()
            .unwrap()
            .ends_with("::ZPreferred")
    );
    let route = plan["routes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|route| route["type"].as_str().unwrap().ends_with("::Store"))
        .unwrap();
    assert_eq!(route["provider"], preferred["id"]);
    assert_eq!(route["projection"], 0);
    assert_eq!(inputs[1]["target"], reports["id"]);
    assert_eq!(inputs[1]["lazy"], true);
    assert_eq!(inputs[2]["optional"], true);
    assert!(inputs[2]["target"].is_null());
    assert!(inputs[2]["projection"].is_null());
    assert_eq!(plan["order"].as_array().unwrap().len(), nodes.len());
    for node in nodes {
        assert!(node.get("primary").is_none());
        assert!(node.get("candidates").is_none());
        assert!(node.get("materialize").is_none());
        assert!(node["adapter"].as_str().is_some());
        assert_eq!(node["adapterCrate"], "reflection_contract");
    }
}
