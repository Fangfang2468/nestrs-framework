//! Resolve help along the command path, before invoking any project tooling.

use std::{ffi::OsString, process::Command};

use super::{before_separator, graph, init, spawn};
use crate::toolchain::cargo_program;

const ROOT_HELP: &str = "Nestrs compiler toolchain

Usage: cargo nestrs <COMMAND> [CARGO OPTIONS] [-- APPLICATION ARGUMENTS]
       cargo nestrs help [COMMAND...]

Commands:
  check    Check Nestrs declarations and generated bindings
  build    Build an application with the Nestrs compiler driver
  run      Build and run an application
  test     Build and run unit and integration tests
  graph    Export project dependency graphs as offline HTML; --bin selects one entry
  init     Initialize an existing project's Nestrs development environment
  doctor   Verify the pinned compiler, rustc-dev and driver installation
  help     Show help for this command or a nested command

Options:
  -h, --help     Show this help
  -V, --version  Show the CLI version

Use cargo nestrs <COMMAND> help or cargo nestrs <COMMAND> --help for command help.
Nested example: cargo nestrs help init check (also init check help / init check --help).
check/build/run/test help uses Cargo's complete option documentation.

Cargo options, including --features, --target, --manifest-path, --locked and
--offline, are forwarded. Arguments after -- are forwarded without changes.
Artifacts use <Cargo target directory>/nestrs/<compiler identity>/<driver+bridge hash>.
Rustc incremental compilation is disabled; Cargo still reuses unchanged artifacts.

NESTRS_RUSTC selects an explicit compiler (its full identity must match the pin).
NESTRS_DRIVER selects an explicit driver; otherwise the sibling nestrs-driver is used.
NESTRS_MACRO_BRIDGE selects the private proc-macro library shipped with the driver.
";

const DOCTOR_HELP: &str = "Usage: cargo nestrs doctor

Verify the pinned compiler identity, rustc-dev libraries, Nestrs compiler driver and
private declaration bridge. Print their paths and the driver fingerprint on success.
This command does not build a project or install toolchain components.

Environment:
  NESTRS_RUSTC         Select an explicit compiler; its full identity must match the pin
  NESTRS_DRIVER        Select a driver instead of the sibling nestrs-driver
  NESTRS_MACRO_BRIDGE  Select the private proc-macro library shipped with the driver

Options:
  -h, --help  Show this help without probing the toolchain

Equivalent help: cargo nestrs help doctor / cargo nestrs doctor help
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Topic {
    Root,
    Cargo(&'static str),
    Graph,
    Init,
    InitCheck,
    Doctor,
}

impl Topic {
    fn child(self, name: &str) -> Option<Self> {
        match (self, name) {
            (Self::Root, "check") => Some(Self::Cargo("check")),
            (Self::Root, "build") => Some(Self::Cargo("build")),
            (Self::Root, "run") => Some(Self::Cargo("run")),
            (Self::Root, "test") => Some(Self::Cargo("test")),
            (Self::Root, "graph") => Some(Self::Graph),
            (Self::Root, "init") => Some(Self::Init),
            (Self::Root, "doctor") => Some(Self::Doctor),
            (Self::Init, "check") => Some(Self::InitCheck),
            _ => None,
        }
    }

    fn path(self) -> &'static str {
        match self {
            Self::Root => "cargo nestrs",
            Self::Cargo("check") => "cargo nestrs check",
            Self::Cargo("build") => "cargo nestrs build",
            Self::Cargo("run") => "cargo nestrs run",
            Self::Cargo(_) => "cargo nestrs test",
            Self::Graph => "cargo nestrs graph",
            Self::Init => "cargo nestrs init",
            Self::InitCheck => "cargo nestrs init check",
            Self::Doctor => "cargo nestrs doctor",
        }
    }

    fn unknown(self, name: &str) -> String {
        if self == Self::Root {
            match name {
                "create" => return "cargo nestrs create is not implemented yet; use cargo nestrs init to initialize an existing project. Run cargo nestrs init --help for usage".into(),
                "ide" => return "cargo nestrs ide has been renamed to cargo nestrs init; rerun init with your previous options to refresh generated editor commands".into(),
                _ => {}
            }
        }
        format!(
            "unknown Nestrs command path {:?}; run {} --help",
            format!("{} {name}", self.path()),
            self.path(),
        )
    }
}

pub(super) fn resolve(args: &[OsString]) -> Result<Option<Topic>, String> {
    if args.is_empty() {
        return Ok(Some(Topic::Root));
    }
    let args = before_separator(args);
    let mut topic = Topic::Root;
    let mut requested = false;
    let mut position = 0;
    while let Some(arg) = args.get(position) {
        if arg == "--help" || arg == "-h" {
            // A flag shows the current level. Only the `help` subcommand walks
            // further (e.g. `init help check` versus `init --help check`).
            return Ok(Some(topic));
        } else if arg == "help" {
            requested = true;
        } else if let Some(name) = arg.to_str() {
            if name.starts_with('-') {
                break;
            }
            if let Some(child) = topic.child(name) {
                topic = child;
            } else if topic == Topic::Cargo("test") && !requested {
                // Cargo test accepts a positional filter; it is not a subcommand.
                break;
            } else {
                return Err(topic.unknown(name));
            }
        } else if topic == Topic::Cargo("test") && !requested {
            break;
        } else {
            return Err("command must be valid UTF-8".into());
        }
        position += 1;
    }
    // Only a leading command path treats the bare word `help` specially. Once
    // options start, names such as `--bin help` and `--output help` are values.
    // Help flags still work after options, but never after the `--` boundary.
    requested |= args[position..]
        .iter()
        .any(|arg| arg == "--help" || arg == "-h");
    Ok(requested.then_some(topic))
}

pub(super) fn show(topic: Topic) -> Result<u8, String> {
    let help = match topic {
        Topic::Root => ROOT_HELP,
        Topic::Graph => graph::HELP,
        Topic::Init => init::HELP,
        Topic::InitCheck => init::CHECK_HELP,
        Topic::Doctor => DOCTOR_HELP,
        // Cargo owns its option list. Normalize every help spelling to the same
        // invocation, without discovering the private compiler or a workspace.
        Topic::Cargo(command) => {
            return spawn(Command::new(cargo_program()).args([command, "--help"]));
        }
    };
    print!("{help}");
    Ok(0)
}
