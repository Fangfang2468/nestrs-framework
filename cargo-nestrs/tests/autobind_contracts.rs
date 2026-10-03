//! 自动绑定的正式编译器契约：有效图逐目标运行，非法图在 check/build 阶段拒绝。
//!
//! fixture 内核对实际查询、实例身份和精确投影；本 harness 另核对编译报告中的需求、
//! 显式绑定和生成投影，避免仅靠程序成功退出遗漏生成阶段的退化。
#![cfg(feature = "compiler-driver")]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use serde_json::Value;

/// 每个正例的接口需求数和显式绑定数，沿用原探针的独立生成断言。
const VALID_CASES: [(&str, usize, usize); 11] = [
    ("positive", 6, 0),
    ("unsatisfied_bound", 1, 0),
    ("explicit", 1, 1),
    ("cfg_selected", 1, 0),
    ("semantic_edges", 3, 0),
    ("unreferenced_generic", 0, 0),
    ("factory_override", 0, 0),
    ("factory_other_key", 1, 0),
    ("source_forms", 3, 0),
    ("explicit_generic_root", 1, 1),
    ("higher_ranked", 3, 0),
];

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/auto-binding")
}

fn artifacts(phase: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("target/autobind-contracts")
        .join(std::process::id().to_string())
        .join(phase);
    fs::create_dir_all(&directory).unwrap();
    directory
}

/// 保留原始业务输入，排除 fixture 的历史 target 缓存。
fn source_snapshot() -> BTreeMap<PathBuf, Vec<u8>> {
    let root = fixture();
    let mut sources = BTreeMap::new();
    for name in ["Cargo.toml", "Cargo.lock"] {
        sources.insert(PathBuf::from(name), fs::read(root.join(name)).unwrap());
    }
    let mut pending = vec![root.join("src")];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else {
                sources.insert(
                    entry.path().strip_prefix(&root).unwrap().to_owned(),
                    fs::read(entry.path()).unwrap(),
                );
            }
        }
    }
    sources
}

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
    command
        .current_dir(fixture())
        .env("CARGO_TARGET_DIR", directory.join("cargo"))
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

fn invoke(command: &mut Command, directory: &Path, label: &str) -> Output {
    let output = command.output().expect("start automatic-binding contract");
    fs::write(
        directory.join(format!("{label}.stdout.log")),
        &output.stdout,
    )
    .unwrap();
    fs::write(
        directory.join(format!("{label}.stderr.log")),
        &output.stderr,
    )
    .unwrap();
    output
}

fn combined(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// 缓存布局由正式工具返回，测试不复制编译器指纹或平台路径算法。
fn compiler_output(directory: &Path) -> PathBuf {
    let output = invoke(
        command(directory)
            .args(["doctor", "--json", "--target-dir"])
            .arg(directory.join("cargo")),
        directory,
        "doctor",
    );
    assert!(output.status.success(), "{}", combined(&output));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    PathBuf::from(report["compiler_output_directory"].as_str().unwrap())
}

fn analysis(root: &Path, binary: &str) -> Value {
    let mut pending = vec![root.to_owned()];
    let mut reports = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                pending.push(entry.path());
            } else if entry.file_name() == "analysis.json" {
                let report: Value =
                    serde_json::from_slice(&fs::read(entry.path()).unwrap()).unwrap();
                if report["crate"] != binary {
                    continue;
                }
                let status: Value = serde_json::from_slice(
                    &fs::read(entry.path().with_file_name("compilation.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(status["passed"], true, "{binary}: {status}");
                assert_eq!(status["passes"], 2, "{binary}: {status}");
                reports.push(report);
            }
        }
    }
    assert_eq!(reports.len(), 1, "{binary}: expected one fresh analysis");
    reports.pop().unwrap()
}

fn verify_valid(phase: &str, cases: &[(&str, usize, usize)], alternate: bool) {
    let before = source_snapshot();
    let directory = artifacts(phase);
    let reports = compiler_output(&directory);
    for &(binary, requests, explicit) in cases {
        let mut build = command(&directory);
        build.args([
            "build",
            "--offline",
            "--locked",
            "--message-format=json",
            "--bin",
            binary,
        ]);
        if alternate {
            build.args(["--features", "alternate"]);
        }
        let output = invoke(&mut build, &directory, &format!("build-{binary}"));
        assert!(output.status.success(), "{binary}: {}", combined(&output));
        let executables = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|event| {
                event["reason"] == "compiler-artifact" && event["target"]["name"] == binary
            })
            .filter_map(|event| event["executable"].as_str().map(PathBuf::from))
            .collect::<Vec<_>>();
        assert_eq!(executables.len(), 1, "{binary}: expected one binary");

        let report = analysis(&reports, binary);
        assert_eq!(report["requests"], requests, "{binary}: {report}");
        assert_eq!(report["explicit_bindings"], explicit, "{binary}: {report}");
        let bindings = report["bindings"].as_array().unwrap();
        assert_eq!(report["generated_bindings"], bindings.len(), "{binary}");
        let pairs = bindings
            .iter()
            .map(|binding| {
                (
                    binding["concrete"].as_str().unwrap(),
                    binding["interface"].as_str().unwrap(),
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            pairs.len(),
            bindings.len(),
            "{binary}: duplicate projection"
        );

        let output = invoke(
            Command::new(&executables[0]).current_dir(fixture()),
            &directory,
            &format!("run-{binary}"),
        );
        assert!(output.status.success(), "{binary}: {}", combined(&output));
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("auto-binding "),
            "{binary}: {}",
            combined(&output)
        );
    }
    assert_eq!(
        source_snapshot(),
        before,
        "compiler modified fixture inputs"
    );
}

#[test]
fn valid_automatic_bindings_preserve_runtime_and_projection_contracts() {
    verify_valid("default", &VALID_CASES, false);
}

#[test]
fn alternate_cfg_selects_only_its_active_provider() {
    verify_valid("alternate", &[("cfg_selected", 1, 0)], true);
}

#[test]
fn invalid_automatic_bindings_fail_check_and_build_before_execution() {
    let before = source_snapshot();
    let directory = artifacts("invalid");
    for (binary, code, message, details) in [
        (
            "ambiguity",
            "NESTRS-DI003",
            "选择唯一",
            &["Consumer", "Port", "First", "Second"][..],
        ),
        (
            "duplicate_explicit",
            "NESTRS-DI009",
            "同一服务与接口被显式绑定多次",
            &["DuplicateBinding", "Service", "Port"][..],
        ),
    ] {
        for operation in ["check", "build"] {
            let output = invoke(
                command(&directory).args([
                    operation,
                    "--offline",
                    "--locked",
                    "--message-format=json",
                    "--bin",
                    binary,
                ]),
                &directory,
                &format!("{operation}-{binary}"),
            );
            let all = combined(&output);
            assert!(!output.status.success(), "{operation} accepted {binary}");
            for forbidden in ["internal compiler error", "panicked at", "must not execute"] {
                assert!(!all.contains(forbidden), "{operation} {binary}: {all}");
            }
            let diagnostics = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .filter(|event| {
                    event["reason"] == "compiler-message" && event["message"]["level"] == "error"
                })
                .map(|event| event["message"].clone())
                .collect::<Vec<_>>();
            assert_eq!(diagnostics.len(), 1, "{operation} {binary}: {all}");
            let title = diagnostics[0]["message"].as_str().unwrap();
            assert!(title.starts_with(&format!("[{code}]")), "{all}");
            assert!(title.contains(message), "{all}");
            let rendered = diagnostics[0]["rendered"].as_str().unwrap();
            for detail in details {
                assert!(rendered.contains(detail), "missing {detail}: {all}");
            }
        }
    }
    assert_eq!(
        source_snapshot(),
        before,
        "compiler modified fixture inputs"
    );
}
