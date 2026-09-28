//! Process-level CLI checks use isolated fake tools; no global toolchain changes.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use cargo_nestrs::toolchain::CompilerIdentity;

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nestrs-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(root.join("sysroot/bin")).unwrap();
        let pinned = CompilerIdentity::pinned().unwrap();
        let libraries = root
            .join("sysroot/lib/rustlib")
            .join(&pinned.host)
            .join("lib");
        fs::create_dir_all(&libraries).unwrap();
        for name in [
            "librustc_middle-test.rmeta",
            "librustc_interface-test.rmeta",
        ] {
            fs::write(libraries.join(name), "").unwrap();
        }
        script(
            &root.join("sysroot/bin/rustc"),
            &format!(
                "case \"$1\" in\n-vV) printf '%s\\n' 'rustc {release}' 'release: {release}' 'commit-hash: {commit}' 'host: {host}' ;;\n--print) printf '%s\\n' '{sysroot}' ;;\n*) exit 90 ;;\nesac\n",
                release = pinned.release,
                commit = pinned.commit,
                host = pinned.host,
                sysroot = root.join("sysroot").display(),
            ),
        );
        script(
            &root.join("nestrs-driver"),
            &format!(
                "printf '%s\\n' '{{\"release\":\"{}\",\"commit_hash\":\"{}\",\"host\":\"{}\"}}'\n",
                pinned.release, pinned.commit, pinned.host,
            ),
        );
        fs::write(
            root.join(bridge_filename()),
            "private proc-macro test artifact",
        )
        .unwrap();
        script(
            &root.join("cargo"),
            "printf '%s\\n' \"$@\" > \"$RECORD_ARGS\"\nprintf '%s\\n' \"$CARGO_TARGET_DIR\" \"$NESTRS_COMPILER_OUTPUT\" \"$RUSTC_WRAPPER\" \"$CARGO_INCREMENTAL\" \"${RUSTC_BOOTSTRAP:-unset}\" \"${NESTRS_GRAPH_TARGET:-unset}\" \"$RUSTDOC\" \"$NESTRS_REAL_RUSTDOC\" \"$NESTRS_MACRO_BRIDGE\" > \"$RECORD_ENV\"\nexit 37\n",
        );
        Self(root)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .env("NESTRS_RUSTC", self.0.join("sysroot/bin/rustc"))
            .env("NESTRS_DRIVER", self.0.join("nestrs-driver"))
            .env("CARGO", self.0.join("cargo"))
            .env("CARGO_TARGET_DIR", self.0.join("target"))
            .env("RECORD_ARGS", self.0.join("args"))
            .env("RECORD_ENV", self.0.join("environment"))
            .env_remove("RUSTC_WRAPPER")
            .env_remove("NESTRS_MACRO_BRIDGE")
            .env_remove("RUSTC_WORKSPACE_WRAPPER");
        command
    }

    fn graph_command(&self, data: &str) -> Command {
        self.graph_command_for(data, "service")
    }

    fn graph_command_for(&self, data: &str, binary: &str) -> Command {
        let package = "path+file:///fixture#app@0.1.0";
        let source = self.0.join("main.rs");
        fs::write(&source, "fn main() {}\n").unwrap();
        let target = serde_json::json!({
            "kind": ["bin"], "name": binary, "src_path": source,
        });
        let metadata = serde_json::json!({
            "workspace_members": [package],
            "workspace_default_members": [package],
            "packages": [{
                "id": package, "name": "app", "version": "0.1.0",
                "manifest_path": self.0.join("Cargo.toml"), "targets": [target],
            }],
        });
        let artifact = serde_json::json!({
            "reason": "compiler-artifact", "package_id": package,
            "target": target, "executable": self.0.join("graph-program"),
        });
        let proof = serde_json::json!({
            "binary": binary, "crate": binary.replace('-', "_"),
            "manifest": self.0, "source": source,
        });
        script(
            &self.0.join("cargo"),
            &format!(
                "if [ \"$1\" = metadata ]; then\n  printf '%s\\n' '{}'\nelse\n  printf '%s\\n' \"$@\" > \"$RECORD_ARGS\"\n  mkdir -p \"${{NESTRS_GRAPH_PROOF%/*}}\"\n  printf '%s\\n' '{}' > \"$NESTRS_GRAPH_PROOF\"\n  printf '%s\\n' '{}'\nfi\n",
                metadata, proof, artifact,
            ),
        );
        script(
            &self.0.join("graph-program"),
            "printf '%s\\n' \"$GRAPH_RESPONSE\"\n",
        );
        let mut command = self.command();
        command.env("GRAPH_RESPONSE", data).arg("graph");
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\nset -eu\n{body}")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn bridge_filename() -> String {
    format!(
        "{}nestrs_tool_bridge{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX,
    )
}

#[test]
fn run_forwards_application_arguments_and_exit_code() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .env("RUSTC_BOOTSTRAP", "1")
        .env("NESTRS_GRAPH_TARGET", "service")
        .args([
            "nestrs",
            "run",
            "--locked",
            "--features",
            "one,two",
            "--",
            "--output",
            "two words",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let environment = fs::read_to_string(fixture.0.join("environment")).unwrap();
    let lines: Vec<_> = environment.lines().collect();
    let namespace = fixture
        .0
        .join("target/nestrs")
        .join(CompilerIdentity::pinned().unwrap().cache_key());
    assert!(Path::new(lines[0]).starts_with(namespace));
    assert_eq!(
        Path::new(lines[1]),
        Path::new(lines[0]).join("nestrs/compiler")
    );
    assert_eq!(Path::new(lines[2]), fixture.0.join("nestrs-driver"));
    assert_eq!(lines[3], "0");
    assert_eq!(lines[4], "unset");
    assert_eq!(lines[5], "unset");
    assert_eq!(Path::new(lines[6]), fixture.0.join("nestrs-driver"));
    assert_eq!(Path::new(lines[7]), fixture.0.join("sysroot/bin/rustdoc"));
    assert_eq!(Path::new(lines[8]), fixture.0.join(bridge_filename()));
    let args = fs::read_to_string(fixture.0.join("args")).unwrap();
    assert_eq!(
        args,
        format!(
            "run\n--locked\n--features\none,two\n--target-dir\n{}\n--\n--output\ntwo words\n",
            lines[0]
        )
    );
}

#[test]
fn doctor_requires_a_matching_full_compiler_identity() {
    let fixture = Fixture::new();
    let output = fixture.command().arg("doctor").output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("Nestrs toolchain is ready"));
    let rustc = fixture.0.join("sysroot/bin/rustc");
    let code = fs::read_to_string(&rustc).unwrap().replace(
        &CompilerIdentity::pinned().unwrap().commit,
        "different-commit",
    );
    fs::write(&rustc, code).unwrap();
    let output = fixture.command().arg("doctor").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("unsupported compiler"));
}

#[test]
fn driver_changes_create_a_new_cargo_cache_namespace() {
    let fixture = Fixture::new();
    let first = fixture.command().arg("build").output().unwrap();
    assert_eq!(first.status.code(), Some(37));
    let old = fs::read_to_string(fixture.0.join("environment")).unwrap();
    let driver = fixture.0.join("nestrs-driver");
    let changed = format!(
        "{}\n# codegen changed\n",
        fs::read_to_string(&driver).unwrap()
    );
    fs::write(driver, changed).unwrap();
    let second = fixture.command().arg("build").output().unwrap();
    assert_eq!(second.status.code(), Some(37));
    let new = fs::read_to_string(fixture.0.join("environment")).unwrap();
    assert_ne!(old.lines().next(), new.lines().next());
}

#[test]
fn bridge_changes_create_a_new_cargo_cache_namespace() {
    let fixture = Fixture::new();
    let first = fixture.command().arg("build").output().unwrap();
    assert_eq!(first.status.code(), Some(37));
    let old = fs::read_to_string(fixture.0.join("environment")).unwrap();
    fs::write(
        fixture.0.join(bridge_filename()),
        "new macro implementation",
    )
    .unwrap();
    let second = fixture.command().arg("build").output().unwrap();
    assert_eq!(second.status.code(), Some(37));
    let new = fs::read_to_string(fixture.0.join("environment")).unwrap();
    assert_ne!(old.lines().next(), new.lines().next());
}

#[test]
fn missing_private_bridge_is_reported_before_cargo_runs() {
    let fixture = Fixture::new();
    fs::remove_file(fixture.0.join(bridge_filename())).unwrap();
    let output = fixture.command().arg("check").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("bridge"));
    assert!(!fixture.0.join("args").exists());
}

#[test]
fn explicit_private_bridge_is_forwarded_to_the_driver() {
    let fixture = Fixture::new();
    let selected = fixture.0.join("selected-bridge.so");
    fs::write(&selected, "selected private bridge").unwrap();
    let output = fixture
        .command()
        .env("NESTRS_MACRO_BRIDGE", &selected)
        .arg("check")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(37));
    let environment = fs::read_to_string(fixture.0.join("environment")).unwrap();
    assert_eq!(Path::new(environment.lines().nth(8).unwrap()), selected);
}

#[test]
fn wrapper_conflicts_are_reported_before_cargo_runs() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .env("RUSTC_WRAPPER", "/some/other/wrapper")
        .arg("check")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("RUSTC_WRAPPER is already configured")
    );
    assert!(!fixture.0.join("args").exists());
}

#[test]
fn doctor_rejects_driver_from_another_compiler() {
    let fixture = Fixture::new();
    let driver = fixture.0.join("nestrs-driver");
    let code = fs::read_to_string(&driver)
        .unwrap()
        .replace(&CompilerIdentity::pinned().unwrap().commit, "old-driver");
    fs::write(driver, code).unwrap();
    let output = fixture.command().arg("doctor").output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("installed compiler driver is incompatible")
    );
}

#[test]
fn doctor_reports_missing_rustc_dev() {
    let fixture = Fixture::new();
    let pinned = CompilerIdentity::pinned().unwrap();
    fs::remove_file(
        fixture
            .0
            .join("sysroot/lib/rustlib")
            .join(pinned.host)
            .join("lib/librustc_middle-test.rmeta"),
    )
    .unwrap();
    let output = fixture.command().arg("doctor").output().unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("rustc-dev is missing"));
}

#[test]
fn target_directory_from_cargo_configuration_is_respected() {
    let fixture = Fixture::new();
    let configured = fixture.0.join("configured-target");
    script(
        &fixture.0.join("cargo"),
        &format!(
            "if [ \"$1\" = metadata ]; then\n  printf '%s\\n' \"$@\" > \"$RECORD_ARGS.metadata\"\n  printf '%s\\n' '{{\"target_directory\":\"{}\"}}'\nelse\n  printf '%s\\n' \"$@\" > \"$RECORD_ARGS\"\n  exit 37\nfi\n",
            configured.display(),
        ),
    );
    let output = fixture
        .command()
        .env_remove("CARGO_TARGET_DIR")
        .args([
            "check",
            "--manifest-path",
            "member/Cargo.toml",
            "--config",
            "build.target-dir='configured-target'",
            "--offline",
            "--locked",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(37),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = fs::read_to_string(fixture.0.join("args.metadata")).unwrap();
    assert_eq!(
        metadata,
        "metadata\n--no-deps\n--format-version\n1\n--manifest-path\nmember/Cargo.toml\n--config\nbuild.target-dir='configured-target'\n--offline\n--locked\n"
    );
    let arguments = fs::read_to_string(fixture.0.join("args")).unwrap();
    let target = arguments.lines().last().unwrap();
    assert!(Path::new(target).starts_with(configured.join("nestrs")));
}

#[test]
fn discovery_only_lists_installed_toolchains_without_installing() {
    let fixture = Fixture::new();
    let pinned = CompilerIdentity::pinned().unwrap();
    script(
        &fixture.0.join("rustup"),
        &format!(
            "printf '%s\\n' \"$@\" >> \"$RECORD_ARGS.rustup\"\nif [ \"$1 $2 $3\" != 'toolchain list --verbose' ]; then exit 91; fi\nprintf '%s\\n' 'stable-{} (active, default) {}'\n",
            pinned.host,
            fixture.0.join("sysroot").display(),
        ),
    );
    let output = fixture
        .command()
        .env_remove("NESTRS_RUSTC")
        .env(
            "PATH",
            std::env::join_paths([
                fixture.0.clone(),
                PathBuf::from("/usr/bin"),
                PathBuf::from("/bin"),
            ])
            .unwrap(),
        )
        .arg("doctor")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("args.rustup")).unwrap(),
        "toolchain\nlist\n--verbose\n"
    );
}

#[test]
fn graph_builds_a_selected_binary_and_writes_offline_html() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("graph.html");
    let output = fixture
        .graph_command(r#"{"version":1,"nodes":[]}"#)
        .args(["-p", "app", "--bin", "service", "--output"])
        .arg(&destination)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = fs::read_to_string(destination).unwrap();
    assert!(html.to_lowercase().starts_with("<!doctype html>"));
    assert!(html.contains("graph-data"));
    let args = fs::read_to_string(fixture.0.join("args")).unwrap();
    assert!(args.starts_with("build\n"));
    assert!(args.contains("--message-format=json"));
    assert!(!args.contains("--output"));
}

#[test]
fn invalid_graph_data_preserves_the_previous_output() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("graph.html");
    fs::write(&destination, "existing graph").unwrap();
    // Project mode writes failure reports; single-entry errors preserve old output.
    let output = fixture
        .graph_command("invalid graph JSON")
        .args(["--bin", "service"])
        .arg("--output")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid graph response"));
    assert_eq!(fs::read_to_string(destination).unwrap(), "existing graph");
}

#[test]
fn graph_export_file_errors_are_reported_and_temporary_files_removed() {
    let fixture = Fixture::new();
    let destination = fixture.0.join("existing-directory.html");
    fs::create_dir_all(&destination).unwrap();
    let output = fixture
        .graph_command(r#"{"version":1,"nodes":[]}"#)
        .arg("--output")
        .arg(&destination)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot export graph"));
    assert!(destination.is_dir());
    assert!(!fs::read_dir(&fixture.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn graph_rejects_foreign_target_before_building() {
    let fixture = Fixture::new();
    let output = fixture
        .graph_command(r#"{"version":1,"nodes":[]}"#)
        .args(["--target", "aarch64-unknown-linux-gnu"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("cross-target graph execution is not supported")
    );
    assert!(!fixture.0.join("args").exists());
}

#[test]
fn graph_does_not_treat_application_help_as_its_own_help() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["graph", "--", "--help"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("takes no arguments after --"));
    assert!(!fixture.0.join("args").exists());
}

#[test]
fn graph_rejects_mixed_targets_before_building() {
    let fixture = Fixture::new();
    for args in [
        vec!["--bins", "--bin", "service"],
        vec!["--test=integration"],
        vec!["--bin=a", "--bin=b"],
        vec!["--message-format=human"],
    ] {
        let output = fixture.command().arg("graph").args(args).output().unwrap();
        assert!(!output.status.success());
        assert!(!fixture.0.join("args").exists());
    }
}

#[test]
fn graph_targets_have_separate_cargo_cache_namespaces() {
    let fixture = Fixture::new();
    let mut namespaces = Vec::new();
    for binary in ["first", "second"] {
        let output = fixture
            .graph_command_for(r#"{"version":1,"nodes":[]}"#, binary)
            .args(["--bin", binary])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let args = fs::read_to_string(fixture.0.join("args")).unwrap();
        let args: Vec<_> = args.lines().collect();
        let target = args
            .windows(2)
            .find(|pair| pair[0] == "--target-dir")
            .unwrap()[1];
        assert!(target.contains(&format!("/graph/{binary}-")));
        namespaces.push(target.to_owned());
    }
    assert_ne!(namespaces[0], namespaces[1]);
}
