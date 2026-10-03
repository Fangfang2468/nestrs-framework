//! 构造方法经真实 Nestrs 工具链处理；独立 fixture 避免污染已有 DI UI 回归入口。
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

fn command(operation: &str, binary: &str) -> Command {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    command
        .args([operation, "--offline", "--manifest-path"])
        .arg(root.join("tests/fixtures/constructors/Cargo.toml"))
        .args(["--bin", binary])
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER");
    command
}

#[test]
fn constructors_preserve_dependency_and_lifetime_semantics_across_crates() {
    for (release, alternate) in [(false, false), (false, true), (true, false)] {
        let mut run = command("run", "valid");
        if release {
            run.arg("--release");
        }
        if alternate {
            run.args(["--features", "alternate"]);
        }
        let output = run.output().expect("启动 constructor 集成 fixture");
        assert!(
            output.status.success(),
            "constructor release={release}, alternate={alternate}\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("constructor contracts passed"));
    }
    let output = command("run", "failures")
        .output()
        .expect("启动 constructor 失败契约 fixture");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("constructor failure contracts passed")
    );
}

#[test]
fn invalid_constructors_fail_during_check_with_contextual_diagnostics() {
    // 不绑定完整 rustc 排版；要求明确拒绝对应 constructor 契约，避免把其他类型错误
    // 或驱动崩溃误当成预期失败。图错误另外核对其类型与相关服务名。
    for (binary, expected) in [
        ("async", &["constructor"][..]),
        ("future", &["constructor"][..]),
        ("receiver", &["constructor"][..]),
        ("trait_impl", &["constructor"][..]),
        ("unsafe", &["constructor"][..]),
        ("extern", &["constructor"][..]),
        ("method_generic", &["constructor"][..]),
        ("duplicate", &["constructor"][..]),
        ("duplicate_marker", &["constructor"][..]),
        ("non_self", &["constructor"][..]),
        ("field_inject", &["constructor"][..]),
        ("field_value", &["constructor"][..]),
        ("arguments", &["constructor"][..]),
        ("orphan", &["constructor"][..]),
        ("free_function", &["constructor"][..]),
        (
            "helper_dependencies",
            &["内部 adapter 和元数据只能由工具链生成代码访问"][..],
        ),
        (
            "helper_activate",
            &["内部 adapter 和元数据只能由工具链生成代码访问"][..],
        ),
        (
            "helper_metadata",
            &["内部 adapter 和元数据只能由工具链生成代码访问"][..],
        ),
        ("error_debug", &["Debug"][..]),
        ("missing", &["MissingDependency", "Missing"][..]),
        ("scope", &["ScopeRequired", "Session"][..]),
        ("cycle", &["Cycle", "First", "Second"][..]),
        ("source_branch_origins", &["constructor"][..]),
        ("source_alias_assignment", &["constructor"][..]),
        ("source_opaque_call", &["constructor"][..]),
        ("source_opaque_return", &["constructor"][..]),
        ("source_update", &["constructor"][..]),
        ("source_loop", &["constructor"][..]),
        ("source_closure_assignment", &["constructor"][..]),
        ("source_tuple_assignment", &["constructor"][..]),
        ("source_unsafe_assignment", &["constructor"][..]),
        ("source_destructured_alias", &["constructor"][..]),
        ("source_foreign_literal", &["constructor"][..]),
    ] {
        let binary = format!("invalid_{binary}");
        let output = command("check", &binary)
            .output()
            .expect("启动 constructor 编译失败 fixture");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{binary} 不应接受非法构造方法");
        assert!(
            !diagnostic.contains("internal compiler error")
                && !diagnostic.contains("custom attribute panicked"),
            "{binary}: {diagnostic}"
        );
        assert!(
            !diagnostic.contains("unresolved import")
                && !diagnostic.contains("cannot find attribute"),
            "{binary} 缺少宏环境不能算构造协议验证成功: {diagnostic}"
        );
        assert!(
            !diagnostic.contains("could not compile `constructor-library`"),
            "{binary} 的上游 fixture 编译失败，尚未验证目标非法声明: {diagnostic}"
        );
        assert!(
            diagnostic
                .replace('\\', "/")
                .contains(&format!("src/bin/{binary}.rs")),
            "{binary} 的诊断没有指向被测入口，不能算预期失败: {diagnostic}"
        );
        // 包名、fixture 路径和 rustc 最后一行也包含 constructor。只查那些文本会
        // 把上游构建失败当成成功拒绝；语法测试必须命中真实错误标题中的契约名称。
        let error_titles = diagnostic
            .lines()
            .filter(|line| {
                (line.starts_with("error:") || line.starts_with("error["))
                    && !line.contains("could not compile")
            })
            .collect::<Vec<_>>()
            .join("\n");
        for expected in expected {
            let message: &str = if *expected == "constructor" {
                &error_titles
            } else {
                &diagnostic
            };
            assert!(
                message.to_lowercase().contains(&expected.to_lowercase()),
                "{binary} 缺少诊断 {expected}: {diagnostic}"
            );
        }
    }
}

#[test]
fn constructor_graph_preserves_parameter_slots_and_private_generic_dependencies() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let output_path = std::env::temp_dir().join(format!(
        "nestrs-constructor-graph-{}-{nonce}.html",
        std::process::id()
    ));
    let output = command("graph", "valid")
        .arg("--output")
        .arg(&output_path)
        .output()
        .expect("导出 constructor fixture 依赖图");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("constructor contracts passed"),
        "图导出不能执行用户 main"
    );
    let html = fs::read_to_string(&output_path).unwrap();
    fs::remove_file(output_path).unwrap();
    let script = html.split_once("id=\"graph-data\"").unwrap().1;
    let json = script
        .split_once('>')
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    let data: serde_json::Value = serde_json::from_str(json).unwrap();
    let graph = if data["version"] == 2 {
        &data["entries"][0]["graph"]
    } else {
        &data
    };
    let nodes = graph["nodes"].as_array().unwrap();
    let node = |label: &str| {
        nodes
            .iter()
            .find(|node| node["label"] == label)
            .unwrap_or_else(|| panic!("依赖图缺少节点 {label}"))
    };
    let checkout = node("Checkout");
    let dependencies = checkout["dependencies"].as_array().unwrap();
    assert_eq!(dependencies.len(), 6);
    for (index, label) in [
        "database_service",
        "clock_service",
        "outbound",
        "codec",
        "absent",
        "reports",
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(dependencies[index]["slot"], index + 1);
        assert_eq!(dependencies[index]["label"], label);
    }
    assert_eq!(
        dependencies[2]["key"],
        serde_json::json!({"kind": "named", "value": "outbound"})
    );
    assert_eq!(
        dependencies[3]["key"],
        serde_json::json!({"kind": "indexed", "value": "7"})
    );
    assert_eq!(dependencies[4]["optional"], true);
    assert!(dependencies[4]["target"].is_null());
    assert_eq!(dependencies[5]["lazy"], true);
    assert_eq!(dependencies[5]["target"], node("Reports")["id"]);
    assert_eq!(dependencies[2]["target"], node("Mailer")["id"]);
    // clock 只参与业务计算，没有被保存在字段中，但仍是构造入口的真实依赖。
    let label_dependencies = node("ValidatedLabel")["dependencies"].as_array().unwrap();
    assert_eq!(label_dependencies.len(), 1);
    assert_eq!(label_dependencies[0]["label"], "clock");
    assert_eq!(label_dependencies[0]["target"], node("Clock")["id"]);
    let options = node("LazyOptions")["dependencies"].as_array().unwrap();
    assert_eq!(options.len(), 3);
    assert!(options.iter().all(|input| input["lazy"] == true));
    assert_eq!(options[0]["target"], node("Mailer")["id"]);
    assert_eq!(options[1]["target"], node("Reports")["id"]);
    assert_eq!(options[2]["optional"], true);
    assert!(options[2]["target"].is_null());

    // 下游无法命名 Inner<Order>，它必须由 Outer<Order> 的构造参数闭合并进图。
    let outer = nodes
        .iter()
        .find(|node| node["name"].as_str().unwrap().contains("::Outer<"))
        .unwrap();
    let inner = nodes
        .iter()
        .find(|node| node["name"].as_str().unwrap().contains("::Inner<"))
        .unwrap();
    assert_eq!(outer["dependencies"][0]["label"], "inner");
    assert_eq!(outer["dependencies"][0]["target"], inner["id"]);
    assert_eq!(inner["dependencies"][0]["target"], node("Clock")["id"]);
}

/// 每次使用独立 HTML 路径；feature 两端都检查冻结图，不能仅凭运行成功推断输入保留。
fn source_flow_graph(alternate: bool) -> serde_json::Value {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "nestrs-source-flow-{}-{nonce}.html",
        std::process::id()
    ));
    let mut command = command("graph", "source_flow");
    command.arg("--output").arg(&path);
    if alternate {
        command.args(["--features", "alternate"]);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = fs::read_to_string(&path).unwrap();
    fs::remove_file(path).unwrap();
    let json = html
        .split_once("id=\"graph-data\"")
        .unwrap()
        .1
        .split_once('>')
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    serde_json::from_str(json).unwrap()
}

#[test]
fn resolved_constructor_sources_obey_cfg_hygiene_and_shadowing() {
    for alternate in [false, true] {
        let mut run = command("run", "source_flow");
        if alternate {
            run.args(["--features", "alternate"]);
        }
        let output = run.output().unwrap();
        assert!(
            output.status.success(),
            "alternate={alternate}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("constructor source flow passed"));
        let graph = source_flow_graph(alternate);
        let nodes = graph["nodes"].as_array().unwrap();
        let node = |label: &str| nodes.iter().find(|node| node["label"] == label).unwrap();
        let conditional = node("Conditional");
        let inputs = conditional["dependencies"].as_array().unwrap();
        assert_eq!(inputs.len(), 2, "cfg 移除存储字段不能移除构造参数输入");
        for (index, label) in ["dependency", "another"].into_iter().enumerate() {
            assert_eq!(inputs[index]["slot"], index + 1);
            assert_eq!(inputs[index]["label"], label);
            assert_eq!(inputs[index]["target"], node("Dependency")["id"]);
        }
        assert_eq!(node("Renamed")["dependencies"].as_array().unwrap().len(), 3);
        for label in ["Unit", "Failing", "Shadowed"] {
            assert_eq!(node(label)["dependencies"].as_array().unwrap().len(), 1);
        }
    }
}
