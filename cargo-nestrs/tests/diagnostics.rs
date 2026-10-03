//! 真实 rustc/Cargo JSON 诊断契约：正文指向业务源码，内部证据留在最后的 cause。
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

fn command(manifest: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    command
        .args([
            "check",
            "--offline",
            "--message-format=json",
            "--manifest-path",
        ])
        .arg(manifest)
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env("CARGO_TERM_COLOR", "never");
    for name in [
        "RUSTC_BOOTSTRAP",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "NESTRS_GRAPH_TARGET",
        "NESTRS_IDE_CAPTURE",
    ] {
        command.env_remove(name);
    }
    command
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn diagnostics(output: &Output) -> Vec<Value> {
    let all = combined_output(output);
    for forbidden in [
        "internal compiler error",
        "panicked at",
        "fatal runtime error",
        "custom attribute panicked",
        "automatic binding 失败",
    ] {
        assert!(!all.contains(forbidden), "异常编译出口 {forbidden}:\n{all}");
    }
    let mut diagnostics = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let event: Value = serde_json::from_str(line)
            .unwrap_or_else(|error| panic!("Cargo JSON 流混入非 JSON 输出：{error}\n{all}"));
        if event["reason"] == "compiler-message" && event["message"]["level"] == "error" {
            let diagnostic = event["message"].clone();
            let title = diagnostic["message"].as_str().unwrap();
            assert!(
                title.starts_with("[NESTRS-DI"),
                "错误没有进入业务诊断出口：{diagnostic}\n{all}"
            );
            diagnostics.push(diagnostic);
        }
    }
    if !output.status.success() {
        assert!(
            !diagnostics.is_empty(),
            "失败没有产生原生 Cargo 诊断：\n{all}"
        );
    }
    diagnostics
}

fn check_shape(diagnostic: &Value, code: &str) {
    let title = diagnostic["message"].as_str().unwrap();
    assert!(title.starts_with(&format!("[{code}]")), "{diagnostic}");
    for forbidden in [
        "__nestrs",
        "ProviderDefinition",
        "FactoryLeaseFrame",
        "Injection<",
        "input_slot",
        "槽位",
        "nestrs_error_",
    ] {
        assert!(
            !title.contains(forbidden),
            "用户标题泄漏内部表示 {forbidden}: {title}"
        );
    }
    let children = diagnostic["children"].as_array().unwrap();
    let cause = children.last().expect("诊断必须有内部 cause");
    assert_eq!(cause["level"], "note", "{diagnostic}");
    assert!(
        cause["message"].as_str().unwrap().starts_with("cause:"),
        "cause 必须在最后：{diagnostic}"
    );
    assert!(
        children[..children.len() - 1]
            .iter()
            .any(|child| child["level"] == "help"),
        "cause 前必须有修复方向：{diagnostic}"
    );
    assert_eq!(
        children
            .iter()
            .filter(|child| child["message"].as_str().unwrap().starts_with("cause:"))
            .count(),
        1
    );
    assert!(
        diagnostic["spans"]
            .as_array()
            .unwrap()
            .iter()
            .any(|span| span["is_primary"] == true),
        "缺少用户主位置：{diagnostic}"
    );
}

fn highlight(span: &Value) -> String {
    span["text"]
        .as_array()
        .unwrap()
        .iter()
        .map(|line| {
            let start = line["highlight_start"].as_u64().unwrap() as usize - 1;
            let end = line["highlight_end"].as_u64().unwrap() as usize - 1;
            line["text"]
                .as_str()
                .unwrap()
                .chars()
                .skip(start)
                .take(end - start)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn all_spans(diagnostic: &Value) -> impl Iterator<Item = &Value> {
    diagnostic["spans"].as_array().unwrap().iter().chain(
        diagnostic["children"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|child| child["spans"].as_array().unwrap()),
    )
}

fn check_complete_primary_type(diagnostic: &Value, expected: &str) {
    assert!(
        diagnostic["spans"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|span| span["is_primary"] == true)
            .any(|span| highlight(span).contains(expected)),
        "注入类型必须完整高亮 `{expected}`，不能只保留首个 token：{diagnostic}"
    );
}

fn check_primary(diagnostic: &Value, source: &Path, patterns: &[&str]) {
    let source_text = fs::read_to_string(source).unwrap();
    let spans = diagnostic["spans"].as_array().unwrap();
    let primary: Vec<_> = spans
        .iter()
        .filter(|span| span["is_primary"] == true)
        .collect();
    let file_name = source.file_name().unwrap().to_str().unwrap();
    assert!(
        primary.iter().any(|span| {
            let path = span["file_name"].as_str().unwrap().replace('\\', "/");
            if !path.ends_with(file_name) || path.contains("/target/") {
                return false;
            }
            let line = span["line_start"].as_u64().unwrap() as usize;
            let Some(text) = source_text.lines().nth(line - 1) else {
                return false;
            };
            // JSON 的正文也必须来自原文件，不能只是把虚拟生成文件的名字改成业务路径。
            let rendered_lines = span["text"].as_array().unwrap();
            if !rendered_lines.iter().enumerate().all(|(offset, rendered)| {
                source_text.lines().nth(line - 1 + offset) == rendered["text"].as_str()
            }) {
                return false;
            }
            patterns.iter().any(|pattern| text.contains(pattern))
                && !highlight(span).trim().is_empty()
        }),
        "主位置没有指向对应用户输入 {source:?} {patterns:?}：{diagnostic}"
    );
}

fn fixture(binary: &str) -> (PathBuf, Output, Vec<Value>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/diagnostics");
    let output = command(&root.join("Cargo.toml"))
        .args(["--bin", binary])
        .output()
        .unwrap();
    let diagnostics = diagnostics(&output);
    (root, output, diagnostics)
}

#[test]
fn all_error_examples_produce_source_first_native_json_diagnostics() {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("example/di-errors");
    // 指向输入行（key 错误指向 key），避免属性宏位置也被误判成准确字段定位。
    let cases: &[(&str, &str, &[&str])] = &[
        (
            "01-missing-concrete",
            "NESTRS-DI001",
            &["database: Database"],
        ),
        (
            "02-missing-trait",
            "NESTRS-DI001",
            &["gateway: dyn PaymentGateway"],
        ),
        (
            "03-missing-default-key",
            "NESTRS-DI002",
            &["database: Database"],
        ),
        (
            "04-missing-named-key",
            "NESTRS-DI002",
            &["#[inject(\"live\")]"],
        ),
        ("05-key-kind-mismatch", "NESTRS-DI002", &["#[inject(7)]"]),
        (
            "06-ambiguous-trait",
            "NESTRS-DI003",
            &["gateway: dyn PaymentGateway"],
        ),
        (
            "07-multiple-primary",
            "NESTRS-DI004",
            &["gateway: dyn PaymentGateway"],
        ),
        (
            "08-primary-other-key",
            "NESTRS-DI003",
            &["gateway: dyn PaymentGateway"],
        ),
        (
            "09-optional-ambiguity",
            "NESTRS-DI003",
            &["sender: Option<dyn MessagePort>"],
        ),
        (
            "10-lazy-ambiguity",
            "NESTRS-DI003",
            &["sender: dyn MessagePort"],
        ),
        ("11-self-cycle", "NESTRS-DI005", &["parent: Cache"]),
        (
            "12-dependency-cycle",
            "NESTRS-DI005",
            &["payment: PaymentService", "order: OrderService"],
        ),
        (
            "13-optional-cycle",
            "NESTRS-DI005",
            &["beta: Option<Beta>", "alpha: Alpha"],
        ),
        (
            "14-lazy-cycle",
            "NESTRS-DI005",
            &["beta: Beta", "alpha: Alpha"],
        ),
        (
            "15-singleton-scoped",
            "NESTRS-DI006",
            &["session: RequestSession"],
        ),
        (
            "16-transitive-scoped",
            "NESTRS-DI006",
            &["formatter: Formatter"],
        ),
        (
            "17-optional-scoped",
            "NESTRS-DI006",
            &["session: Option<dyn SessionPort>"],
        ),
        (
            "18-lazy-scoped",
            "NESTRS-DI006",
            &["intermediate: Intermediate"],
        ),
        (
            "19-factory-missing",
            "NESTRS-DI001",
            &["fn application(_database: Database)"],
        ),
        (
            "20-factory-cycle",
            "NESTRS-DI005",
            &["fn first(_second: Second)", "fn second(_first: First)"],
        ),
        (
            "21-factory-scoped",
            "NESTRS-DI006",
            &["fn application(_session: RequestSession)"],
        ),
        (
            "22-factory-lazy-missing",
            "NESTRS-DI001",
            &["fn report_service(#[lazy] missing: Missing)"],
        ),
        (
            "23-constructor-missing",
            "NESTRS-DI001",
            &["fn new(_database: Database)"],
        ),
        (
            "24-duplicate-provider",
            "NESTRS-DI007",
            &["fn database()", "fn fallback_database()"],
        ),
        (
            "25-generic-missing",
            "NESTRS-DI001",
            &["database: Database"],
        ),
        (
            "26-generic-growth",
            "NESTRS-DI008",
            &["growing::<(T,)>(provider)"],
        ),
        (
            "27-duplicate-binding",
            "NESTRS-DI009",
            &["#[bind]", "impl Port for Service"],
        ),
        (
            "28-orphan-binding",
            "NESTRS-DI010",
            &["#[bind]", "impl Port for Service"],
        ),
        (
            "29-provider-lazy-missing",
            "NESTRS-DI001",
            &["missing: Missing"],
        ),
    ];
    for &(name, code, patterns) in cases {
        let root = examples.join(name);
        let output = command(&root.join("Cargo.toml")).output().unwrap();
        assert!(!output.status.success(), "{name} 不应编译成功");
        let errors = diagnostics(&output);
        assert_eq!(
            errors.len(),
            1,
            "{name} 同一根因不能按编译阶段重复报告：{}",
            combined_output(&output)
        );
        check_shape(&errors[0], code);
        check_primary(&errors[0], &root.join("src/main.rs"), patterns);
        let complete_type = match name {
            "02-missing-trait"
            | "06-ambiguous-trait"
            | "07-multiple-primary"
            | "08-primary-other-key" => Some("dyn PaymentGateway"),
            "09-optional-ambiguity" | "10-lazy-ambiguity" => Some("dyn MessagePort"),
            "17-optional-scoped" => Some("dyn SessionPort"),
            _ => None,
        };
        if let Some(expected) = complete_type {
            check_complete_primary_type(&errors[0], expected);
        }
        if name == "25-generic-missing" {
            assert!(
                all_spans(&errors[0]).any(|span| highlight(span).contains("Repository<User>")),
                "闭合实例来源必须包含完整类型参数：{}",
                errors[0]
            );
        }
        // 参数/字段错误不能把整条生成签名或包围它的宏属性当作类型位置。
        if !matches!(
            code,
            "NESTRS-DI007" | "NESTRS-DI008" | "NESTRS-DI009" | "NESTRS-DI010"
        ) {
            let spans = errors[0]["spans"].as_array().unwrap();
            for primary in spans.iter().filter(|span| span["is_primary"] == true) {
                let token = highlight(primary);
                assert_eq!(
                    primary["line_start"], primary["line_end"],
                    "{name}: {primary}"
                );
                assert!(
                    !token.contains("#[") && !token.contains("fn ") && !token.contains("struct "),
                    "{name} 未精确定位注入类型或 key：{token}"
                );
            }
        }
    }
}

#[test]
fn independent_errors_remain_separate_and_ordered_by_user_source() {
    let (root, output, errors) = fixture("independent_missing");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 2, "{}", combined_output(&output));
    for (error, (service, input)) in errors.iter().zip([
        ("First", "dependency: Database"),
        ("Second", "dependency: Queue"),
    ]) {
        check_shape(error, "NESTRS-DI001");
        assert!(
            error["message"].as_str().unwrap().contains(service),
            "{error}"
        );
        check_primary(
            error,
            &root.join("src/bin/independent_missing.rs"),
            &[input],
        );
    }
}

#[test]
fn macro_generated_fields_report_the_original_invocation_or_preserved_token() {
    let (root, output, errors) = fixture("macro_missing");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI001");
    check_primary(
        &errors[0],
        &root.join("src/bin/macro_missing.rs"),
        &["declare_service!(Application, Missing)"],
    );
    let primary = errors[0]["spans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|span| span["is_primary"] == true)
        .unwrap();
    assert!(highlight(primary).contains("Missing"), "{primary}");
}

#[test]
fn names_and_use_aliases_never_replace_real_type_identity() {
    let (root, output, errors) = fixture("same_names_missing");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI001");
    check_primary(
        &errors[0],
        &root.join("src/bin/same_names_missing.rs"),
        &["missing: RequestedDatabase"],
    );
    assert!(!errors[0]["message"].as_str().unwrap().contains("ready"));

    let (root, output, errors) = fixture("alias_duplicate");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI007");
    check_primary(
        &errors[0],
        &root.join("src/bin/alias_duplicate.rs"),
        &["fn first()", "fn second()"],
    );
    let spans = errors[0]["spans"].as_array().unwrap();
    assert!(
        spans.iter().any(|span| span["is_primary"] == false),
        "冲突必须关联另一处创建声明：{}",
        errors[0]
    );

    let (root, output, errors) = fixture("constructor_same_name");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI001");
    check_primary(
        &errors[0],
        &root.join("src/bin/constructor_same_name.rs"),
        &["fn Application(_database: Database)"],
    );
    let title = errors[0]["message"].as_str().unwrap();
    assert!(
        title.contains("Application::Application") && title.contains("参数 `_database`"),
        "constructor 角色不能靠方法名是否等于类型名推断：{title}"
    );
    let primary = errors[0]["spans"]
        .as_array()
        .unwrap()
        .iter()
        .find(|span| span["is_primary"] == true)
        .unwrap();
    assert_eq!(
        highlight(primary),
        "Database",
        "同名 constructor 仍须精确定位参数类型：{primary}"
    );
}

#[test]
fn cross_crate_generic_diagnostics_preserve_field_and_closed_use_origins() {
    let (root, output, errors) = fixture("cross_crate_generic");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI001");
    check_primary(
        &errors[0],
        &root.join("library/src/lib.rs"),
        &["database: Database"],
    );
    let title = errors[0]["message"].as_str().unwrap();
    assert!(
        title.contains("Repository") && title.contains("User"),
        "{title}"
    );
    assert!(
        all_spans(&errors[0]).any(|span| {
            span["file_name"]
                .as_str()
                .unwrap()
                .replace('\\', "/")
                .ends_with("src/bin/cross_crate_generic.rs")
                && highlight(span).contains("Repository<User>")
        }),
        "闭合实例必须关联触发它的用户输入：{}",
        errors[0]
    );
}

#[test]
fn root_query_ambiguity_has_a_real_query_location_without_any_input_field() {
    let (root, output, errors) = fixture("root_query_ambiguity");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI003");
    check_primary(
        &errors[0],
        &root.join("src/bin/root_query_ambiguity.rs"),
        &["provider.get_service::<dyn Port>()"],
    );
    check_complete_primary_type(&errors[0], "dyn Port");
    let title = errors[0]["message"].as_str().unwrap();
    assert!(title.contains("Port"), "{title}");
    let rendered = errors[0]["rendered"].as_str().unwrap();
    assert!(
        rendered.contains("First") && rendered.contains("Second"),
        "{rendered}"
    );
}

#[test]
fn cfg_excluded_macro_fields_do_not_create_diagnostic_dependencies() {
    let (_, output, errors) = fixture("cfg_valid");
    assert!(output.status.success(), "{}", combined_output(&output));
    assert!(errors.is_empty());
}

#[test]
fn finite_type_complexity_guard_keeps_the_original_dependency_location() {
    let (root, output, errors) = fixture("finite_complex_type");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI008");
    check_primary(
        &errors[0],
        &root.join("src/bin/finite_complex_type.rs"),
        &["dependency: Holder<Large>"],
    );
    check_complete_primary_type(&errors[0], "Holder<Large>");
    let cause = errors[0]["children"].as_array().unwrap().last().unwrap()["message"]
        .as_str()
        .unwrap();
    assert!(
        cause.contains("1024"),
        "复杂度阈值保留在内部 cause：{cause}"
    );
}

#[test]
fn unused_explicit_ambiguity_points_to_a_real_candidate_without_inventing_a_query() {
    let (root, output, errors) = fixture("unused_explicit_ambiguity");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI003");
    check_primary(
        &errors[0],
        &root.join("src/bin/unused_explicit_ambiguity.rs"),
        &["struct Alpha", "struct Beta"],
    );
    let title = errors[0]["message"].as_str().unwrap();
    assert!(
        title.contains("Port") && !title.contains("查询"),
        "没有输入或查询时必须如实说明声明冲突：{title}"
    );
    let rendered = errors[0]["rendered"].as_str().unwrap();
    assert!(
        rendered.contains("Alpha") && rendered.contains("Beta"),
        "{rendered}"
    );
}

#[test]
fn long_cycles_keep_real_edges_and_write_a_readable_complete_cause() {
    let (root, output, errors) = fixture("long_cycle");
    assert!(!output.status.success());
    assert_eq!(errors.len(), 1, "{}", combined_output(&output));
    check_shape(&errors[0], "NESTRS-DI005");
    check_primary(
        &errors[0],
        &root.join("src/bin/long_cycle.rs"),
        &["=> LongCycleService"],
    );
    let children = errors[0]["children"].as_array().unwrap();
    let path = children
        .iter()
        .filter_map(|child| child["message"].as_str())
        .find(|message| message.starts_with("依赖环："))
        .expect("正文必须解释闭环路径");
    let omitted: usize = path
        .split_once("已省略 ")
        .and_then(|(_, tail)| tail.split_whitespace().next())
        .expect("摘要必须明确省略关系的数量")
        .parse()
        .unwrap();
    assert!(omitted > 0 && omitted < 12, "{path}");
    let mut shown = std::collections::BTreeSet::new();
    for line in path.lines() {
        let Some((consumer, target)) = line.split_once(" → ") else {
            continue;
        };
        let source = consumer.strip_suffix(".dependency").unwrap();
        let target = target.split_whitespace().next().unwrap();
        let number: usize = source
            .strip_prefix("LongCycleService")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(
            target,
            format!("LongCycleService{:02}", number % 12 + 1),
            "省略中间节点不能伪造直连边：{path}"
        );
        assert!(
            shown.insert((source, target)),
            "摘要不能重复同一真实输入边：{path}"
        );
    }
    assert_eq!(
        shown.len() + omitted,
        12,
        "省略数量必须对应真实闭环：{path}"
    );

    let cause = children.last().unwrap()["message"].as_str().unwrap();
    let artifact = cause
        .split_once("完整 cause：")
        .map(|(_, path)| PathBuf::from(path.trim()))
        .expect("超过终端长度的 cause 必须给出完整详情工件路径");
    let artifact = if artifact.is_absolute() {
        artifact
    } else {
        root.join(artifact)
    };
    let complete = fs::read_to_string(&artifact)
        .unwrap_or_else(|error| panic!("完整 cause 工件不可读取 {artifact:?}: {error}"));
    assert!(
        complete.chars().count() > 1800,
        "fixture 必须真实触发超长 cause 分支"
    );
    assert!(complete.starts_with("Cycle\n"), "{complete}");
    for number in 1..=12 {
        assert!(
            complete.contains(&format!("LongCycleService{number:02}")),
            "完整 cause 遗漏了第 {number} 个节点"
        );
    }
}
