//! Compile the original UI contracts using the toolchain's private macro bridge.
//!
//! Legacy stderr files are kept unchanged. Standalone Cargo has different source
//! frames, so compare structured diagnostic codes/messages and their multiplicity.
//! Both unexpected and missing errors fail; a compiler failure alone never passes.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde_json::Value;

type Diagnostics = BTreeMap<(Option<String>, String), usize>;

fn normalized_message(message: &str) -> String {
    // Standalone Cargo targets may print fully qualified names where trybuild
    // used short names. These exact aliases denote the same diagnostic types.
    message
        .replace("`std::fmt::Debug`", "`Debug`")
        .replace("nestrs_core::ServiceProvider", "ServiceProvider")
}

fn quoted(path: &Path) -> String {
    serde_json::to_string(path.to_str().expect("fixture paths must be UTF-8")).unwrap()
}

fn sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .collect();
    sources.sort();
    sources
}

fn expected_errors(path: &Path) -> Diagnostics {
    let mut expected = Diagnostics::new();
    let baseline = fs::read_to_string(path.with_extension("stderr")).unwrap();
    for line in baseline.lines() {
        let diagnostic = if let Some(message) = line.strip_prefix("error: ") {
            Some((None, message))
        } else if let Some(line) = line.strip_prefix("error[") {
            line.split_once("]: ")
                .map(|(code, message)| (Some(code.to_owned()), message))
        } else {
            None
        };
        if let Some((code, message)) = diagnostic {
            // Keep the original baseline: this one public namespace was removed,
            // while the contract that register! is absent remains unchanged.
            let message = if path.file_stem().unwrap() == "register-is-not-exported" {
                message.replace("`nestrs_macro`", "`nestrs`")
            } else {
                message.to_owned()
            };
            *expected.entry((code, message)).or_default() += 1;
        }
    }
    assert!(!expected.is_empty(), "no error contract in {path:?}");
    expected
}

#[test]
fn macro_declarations_preserve_all_ui_contracts() {
    assert!(
        std::env::var_os("RUSTC_WRAPPER").is_some()
            && std::env::var_os("NESTRS_MACRO_BRIDGE").is_some(),
        "run UI regressions with cargo nestrs test so every case receives the private macro bridge"
    );
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let core = fixture
        .join("../../../../nestrs-core")
        .canonicalize()
        .unwrap();
    let output = fixture.join("../../../../target/nestrs-ui");
    let project = output.join("project");
    let records = output.join("diagnostics");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&records).unwrap();

    let passing = sources(&fixture.join("tests/ui/pass"));
    let failing = sources(&fixture.join("tests/ui/fail"));
    let macro_passing = sources(&fixture.join("tests/ui/macro-pass"));
    let macro_failing = sources(&fixture.join("tests/ui/macro-fail"));
    assert_eq!(passing.len(), 15, "preserve every original passing case");
    assert_eq!(failing.len(), 37, "preserve every original failing case");
    assert_eq!(
        macro_passing.len(),
        1,
        "check renamed ordinary macro imports"
    );
    assert_eq!(
        macro_failing.len(),
        5,
        "check ordinary macro/helper misuse and reject named injection keys"
    );

    let mut manifest = format!(
        "[package]\nname = \"nestrs-macro-ui\"\nversion = \"0.0.0\"\nedition = \"2024\"\npublish = false\n\n[workspace]\n\n[dependencies]\nnestrs-core = {{ path = {} }}\ntokio = {{ version = \"1.53.1\", features = [\"rt-multi-thread\", \"macros\"] }}\n",
        quoted(&core),
    );
    let cases: Vec<_> = passing
        .iter()
        .map(|path| (true, path))
        .chain(macro_passing.iter().map(|path| (true, path)))
        .chain(failing.iter().map(|path| (false, path)))
        .chain(macro_failing.iter().map(|path| (false, path)))
        .map(|(pass, path)| {
            let name = format!(
                "ui_{}_{}",
                if pass { "pass" } else { "fail" },
                path.file_stem()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .replace('-', "_")
            );
            manifest.push_str(&format!(
                "\n[[bin]]\nname = {name:?}\npath = {}\n",
                quoted(path),
            ));
            (pass, path, name)
        })
        .collect();
    let manifest_path = project.join("Cargo.toml");
    fs::write(&manifest_path, manifest).unwrap();
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    // Cargo does not fingerprint arbitrary wrapper binary contents. Recompile
    // this tiny fixture package on every run while retaining dependency builds.
    assert!(
        Command::new(&cargo)
            .args(["clean", "--manifest-path"])
            .arg(&manifest_path)
            .arg("--target-dir")
            .arg(output.join("build"))
            .args(["-p", "nestrs-macro-ui"])
            .status()
            .unwrap()
            .success()
    );
    let mut failures = Vec::new();
    for (pass, path, name) in cases {
        let result = Command::new(&cargo)
            .args(["build", "--message-format=json", "--manifest-path"])
            .arg(&manifest_path)
            .arg("--target-dir")
            .arg(output.join("build"))
            .args(["--bin", &name])
            .env("CARGO_INCREMENTAL", "0")
            .output()
            .expect("start Cargo for a macro UI case");
        fs::write(records.join(format!("{name}.jsonl")), &result.stdout).unwrap();
        fs::write(records.join(format!("{name}.stderr")), &result.stderr).unwrap();
        let mut actual = Diagnostics::new();
        let mut executable = None;
        let mut misuse_spans_valid = true;
        for line in String::from_utf8_lossy(&result.stdout).lines() {
            let Ok(message) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if message["reason"] == "compiler-message" && message["message"]["level"] == "error" {
                let diagnostic = &message["message"];
                let text = diagnostic["message"].as_str().unwrap();
                if !text.starts_with("aborting due to") {
                    let code = diagnostic["code"]["code"].as_str().map(str::to_owned);
                    *actual.entry((code, normalized_message(text))).or_default() += 1;
                    if path.parent().unwrap().ends_with("macro-fail") {
                        let expected_line = match path.file_stem().and_then(|stem| stem.to_str()) {
                            Some(
                                "inject-rejects-named-field-key"
                                | "inject-rejects-named-factory-key",
                            ) => 7,
                            _ => 1,
                        };
                        misuse_spans_valid &= diagnostic["spans"].as_array().is_some_and(|spans| {
                            spans.iter().any(|span| {
                                span["is_primary"] == true
                                    && span["line_start"] == expected_line
                                    && span["file_name"].as_str().is_some_and(|file| {
                                        Path::new(file).file_name() == path.file_name()
                                    })
                            })
                        });
                    }
                }
            }
            if message["reason"] == "compiler-artifact" && message["target"]["name"] == name {
                executable = message["executable"].as_str().map(PathBuf::from);
            }
        }
        let label = path.file_name().unwrap().to_str().unwrap();
        if !misuse_spans_valid {
            failures.push(format!(
                "{label}: misuse error must point to the original attribute"
            ));
        }
        if pass {
            if !result.status.success() || !actual.is_empty() {
                failures.push(format!(
                    "{label}: expected successful compilation; got {actual:?}"
                ));
            } else if let Some(executable) = executable {
                if !Command::new(executable).status().unwrap().success() {
                    failures.push(format!("{label}: compiled program failed"));
                }
            } else {
                failures.push(format!("{label}: Cargo did not report a linked executable"));
            }
        } else {
            let expected = expected_errors(path);
            if result.status.success() || actual != expected {
                failures.push(format!(
                    "{label}: expected {expected:?}; got {actual:?}; status {}",
                    result.status
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "macro UI contract mismatches:\n{}\nfull diagnostics: {}",
        failures.join("\n"),
        records.display()
    );
}
