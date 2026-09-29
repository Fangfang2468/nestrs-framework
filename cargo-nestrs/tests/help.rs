//! Help routing must work before a project or the Nestrs toolchain exists.

use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct EmptyWorkspace(PathBuf);

impl EmptyWorkspace {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "nestrs-help-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir_all(&directory).unwrap();
        Self(directory)
    }

    fn invoke(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
            .current_dir(&self.0)
            .env("CARGO", self.0.join("missing-cargo"))
            .env("NESTRS_RUSTC", self.0.join("missing-rustc"))
            .env("NESTRS_DRIVER", self.0.join("missing-driver"))
            .env("NESTRS_MACRO_BRIDGE", self.0.join("missing-bridge"))
            .args(args)
            .output()
            .unwrap()
    }

    fn help(&self, args: &[&str], usage: &str) -> Vec<u8> {
        let output = self.invoke(args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(output.stderr.is_empty(), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(usage),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stdout),
        );
        assert_eq!(fs::read_dir(&self.0).unwrap().count(), 0);
        output.stdout
    }
}

impl Drop for EmptyWorkspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn root_help_aliases_work_without_a_workspace_or_toolchain() {
    let workspace = EmptyWorkspace::new();
    let usage = "Usage: cargo nestrs <COMMAND>";
    let expected = workspace.help(&[], usage);
    for args in [
        vec!["help"],
        vec!["--help"],
        vec!["-h"],
        vec!["nestrs", "help"],
    ] {
        assert_eq!(workspace.help(&args, usage), expected, "{args:?}");
    }
}

#[test]
fn command_help_aliases_select_the_same_topic_without_running_the_command() {
    let workspace = EmptyWorkspace::new();
    for command in ["graph", "init", "doctor"] {
        let usage = format!("Usage: cargo nestrs {command}");
        let expected = workspace.help(&["help", command], &usage);
        for args in [
            vec![command, "help"],
            vec![command, "--help"],
            vec![command, "-h"],
            vec!["nestrs", command, "help"],
        ] {
            assert_eq!(workspace.help(&args, &usage), expected, "{args:?}");
        }
    }
}

#[test]
fn init_check_help_is_a_distinct_nested_topic() {
    let workspace = EmptyWorkspace::new();
    let usage = "Usage: cargo nestrs init check";
    let expected = workspace.help(&["help", "init", "check"], usage);
    for args in [
        vec!["init", "help", "check"],
        vec!["init", "check", "help"],
        vec!["init", "check", "--help"],
        vec!["init", "check", "-h"],
        vec!["nestrs", "help", "init", "check"],
    ] {
        assert_eq!(workspace.help(&args, usage), expected, "{args:?}");
    }
    assert_ne!(
        expected,
        workspace.help(&["init", "help"], "Usage: cargo nestrs init"),
    );
}

#[test]
fn help_flags_after_context_options_do_not_load_the_project_or_toolchain() {
    let workspace = EmptyWorkspace::new();
    let graph = workspace.help(&["graph", "help"], "Usage: cargo nestrs graph");
    let check = workspace.help(&["init", "check", "help"], "Usage: cargo nestrs init check");
    for flag in ["--help", "-h"] {
        assert_eq!(
            workspace.help(
                &["graph", "-p", "missing", flag],
                "Usage: cargo nestrs graph"
            ),
            graph,
        );
        assert_eq!(
            workspace.help(
                &["init", "check", "--output", "missing", flag],
                "Usage: cargo nestrs init check",
            ),
            check,
        );
    }
}

#[test]
fn a_help_flag_selects_the_current_command_before_later_arguments() {
    let workspace = EmptyWorkspace::new();
    let expected = workspace.help(&["init", "help"], "Usage: cargo nestrs init");
    for flag in ["--help", "-h"] {
        assert_eq!(
            workspace.help(&["init", flag, "check"], "Usage: cargo nestrs init"),
            expected,
        );
    }
}

#[test]
fn init_rejects_help_after_the_separator_as_a_program_argument() {
    let workspace = EmptyWorkspace::new();
    for args in [
        vec!["init", "--", "--help"],
        vec!["init", "check", "--", "--help"],
    ] {
        let output = workspace.invoke(&args);
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("does not accept program arguments"),
            "{args:?}: {diagnostic}",
        );
    }
    assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 0);
}

#[test]
fn unknown_help_paths_fail_before_loading_the_toolchain() {
    let workspace = EmptyWorkspace::new();
    for args in [
        vec!["help", "unknown-topic"],
        vec!["unknown-topic", "help"],
        vec!["unknown-topic", "--help"],
        vec!["help", "init", "unknown-topic"],
        vec!["init", "help", "unknown-topic"],
        vec!["init", "unknown-topic", "help"],
        vec!["init", "check", "help", "unknown-topic"],
        vec!["graph", "unknown-topic", "--help"],
    ] {
        let output = workspace.invoke(&args);
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("unknown-topic"),
            "{args:?}: {diagnostic}"
        );
        assert!(!diagnostic.contains("missing-"), "{args:?}: {diagnostic}");
    }
    assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 0);
}

#[test]
fn create_help_reports_that_scaffolding_is_not_implemented() {
    let workspace = EmptyWorkspace::new();
    for args in [
        vec!["help", "create"],
        vec!["create", "help"],
        vec!["create", "--help"],
        vec!["create", "-h"],
    ] {
        let output = workspace.invoke(&args);
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{args:?}: {output:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            diagnostic.contains("not implemented"),
            "{args:?}: {diagnostic}"
        );
        assert!(
            diagnostic.contains("cargo nestrs init"),
            "{args:?}: {diagnostic}"
        );
    }
    assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 0);
}
