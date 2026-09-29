//! clap owns the command tree and Nestrs options; Cargo owns its argument grammar.

use std::{ffi::OsString, path::PathBuf};

use clap::{
    Arg, ArgAction, Command, Error,
    builder::{OsStringValueParser, TypedValueParser},
    error::ErrorKind,
};

const CARGO_COMMANDS: &[(&str, &str)] = &[
    ("check", "Check Nestrs declarations and generated bindings"),
    (
        "build",
        "Build an application with the Nestrs compiler driver",
    ),
    ("run", "Build and run an application"),
    ("test", "Build and run unit and integration tests"),
];

#[derive(Debug, PartialEq, Eq)]
pub(super) struct GraphOptions {
    pub output: Option<PathBuf>,
    pub cargo_args: Vec<OsString>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) struct InitOptions {
    pub check: bool,
    pub vscode: bool,
    pub output: Option<PathBuf>,
    pub cargo_args: Vec<OsString>,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Invocation {
    Doctor,
    Graph(GraphOptions),
    Init(InitOptions),
    Cargo {
        command: String,
        args: Vec<OsString>,
    },
    CargoHelp(String),
}

fn cargo_arguments() -> Arg {
    Arg::new("cargo_args")
        .value_name("CARGO OPTIONS")
        .num_args(0..)
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new())
        .help("Forward options unchanged to Cargo")
}

fn output_argument(help: &'static str) -> Arg {
    Arg::new("output")
        .long("output")
        .value_name("PATH")
        .allow_hyphen_values(true)
        .value_parser(OsStringValueParser::new().try_map(|value| {
            if value.is_empty() {
                Err("output path must not be empty")
            } else {
                Ok(PathBuf::from(value))
            }
        }))
        .help(help)
}

fn vscode_argument() -> Arg {
    Arg::new("vscode")
        .long("vscode")
        .action(ArgAction::SetTrue)
        .help("Merge generated settings into .vscode/settings.json, preserving existing settings and comments")
}

fn command_node(name: &'static str) -> Command {
    Command::new(name).disable_help_flag(true).arg(
        Arg::new("help")
            .short('h')
            .long("help")
            .action(ArgAction::HelpLong)
            .help("Show help for this command"),
    )
}

/// Adding a command here also adds its usage, help route and option metadata.
fn command() -> Command {
    command_node("cargo-nestrs")
        .bin_name("cargo nestrs")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand_required(true)
        .color(clap::ColorChoice::Never)
        .about("Nestrs compiler toolchain")
        .subcommands(CARGO_COMMANDS.iter().map(|&(name, about)| {
            command_node(name).about(about).arg(cargo_arguments())
        }))
        .subcommand(
            command_node("graph")
                .about("Export project dependency graphs as offline HTML")
                .long_about("Without --bin, export every binary in the selected package (or --workspace).\nEach entry is built and validated independently. Disabled required features are\nreported as skipped; errors stay visible in the report and return a nonzero status.\n\nWith --bin, export only that binary's validated graph. Failure preserves previous\noutput. Both modes avoid the business main and service constructors.")
                .args_override_self(true)
                .arg(output_argument("Write HTML here; default: <Cargo target directory>/nestrs-di.html"))
                .arg(cargo_arguments())
                .after_help("Cargo selectors: -p/--package PACKAGE, --workspace, --bin NAME.\nUse -p PACKAGE --features FEATURES for package-specific features;\n--workspace --features is not supported. --all-features applies per package.\n\nEquivalent help: cargo nestrs help graph / cargo nestrs graph help"),
        )
        .subcommand(
            command_node("init")
                .about("Initialize an existing project's Nestrs development environment")
                .long_about("Check the selected targets and generate rust-analyzer project data and client\nsettings using the same compiler, features, cfg and private declaration bridge\nas cargo nestrs check. Application Cargo.toml and source files are not modified;\nno project is created. Rerun init to refresh after changing features or tools.")
                .args_override_self(true)
                .arg(output_argument("Write the project model here; default: <Cargo target>/nestrs/ide/rust-project.json"))
                .arg(vscode_argument())
                .arg(cargo_arguments())
                .subcommand(
                    command_node("check")
                        .about("Emit Cargo JSON diagnostics and refresh the project model after a successful check")
                        .long_about("Generated check-on-save commands use this entry automatically. Emit Cargo JSON\ndiagnostics and refresh the rust-analyzer project model after a successful check.\nA failed check preserves the last successful model.")
                        .args_override_self(true)
                        .arg(output_argument("Write the project model here; default: <Cargo target>/nestrs/ide/rust-project.json"))
                        // Keep this reserved Nestrs option recognizable instead of
                        // accidentally forwarding it as an unknown Cargo option.
                        .arg(vscode_argument().hide(true))
                        .arg(cargo_arguments())
                        .after_help("By default all workspace targets are checked. Cargo selection, feature and profile\nflags are forwarded. Use cargo check --help for the complete Cargo option list.\nThis command does not modify editor settings and does not accept --vscode.\nFor initial setup, use cargo nestrs init (or cargo nestrs init --vscode).\n\nEquivalent help: cargo nestrs help init check / cargo nestrs init check help"),
                )
                .after_help("By default all workspace targets are included. Cargo selection and feature flags\nare forwarded. Client settings are written to adjacent rust-analyzer-settings.json;\nload these in your LSP client. Original source files remain live, including unsaved edits.\n\nUse cargo nestrs init check help for check-on-save options."),
        )
        .subcommand(
            command_node("doctor")
                .about("Verify the pinned compiler, rustc-dev and driver installation")
                .long_about("Verify the pinned compiler identity, rustc-dev libraries, compiler driver and\nprivate declaration bridge. Print their paths and the driver fingerprint on success.\nThis command does not build a project or install toolchain components.")
                .after_help("NESTRS_RUSTC selects an explicit compiler; its full identity must match the pin.\nNESTRS_DRIVER selects a driver instead of the sibling nestrs-driver.\nNESTRS_MACRO_BRIDGE selects the private proc-macro library shipped with the driver.\n\nEquivalent help: cargo nestrs help doctor / cargo nestrs doctor help"),
        )
        .after_help("Use cargo nestrs help <COMMAND> or cargo nestrs <COMMAND> help for command help.\nNested example: cargo nestrs help init check (also init check help / init check --help).\ncheck/build/run/test help uses Cargo's complete option documentation.\n\nCargo options are forwarded, including --features, --target, --manifest-path,\n--locked and --offline. Arguments after -- are forwarded unchanged.\nArtifacts: <Cargo target directory>/nestrs/<compiler identity>/<driver+bridge hash>.\nRustc incremental compilation is disabled; Cargo still reuses unchanged artifacts.\n\nNESTRS_RUSTC selects a compiler whose full identity must match the pin.\nNESTRS_DRIVER selects a driver instead of the sibling nestrs-driver.\nNESTRS_MACRO_BRIDGE selects the private proc-macro library shipped with the driver.")
}

struct Route<'a> {
    command: &'a Command,
    path: Vec<OsString>,
    consumed: usize,
    help: bool,
}

/// Normalize the established `help` spellings using the actual clap tree. Stop
/// at options or a test filter, so a value named `help` remains an argument.
fn route<'a>(root: &'a Command, args: &[OsString]) -> Result<Route<'a>, Error> {
    let mut route = Route {
        command: root,
        path: Vec::new(),
        consumed: 0,
        help: args.is_empty(),
    };
    for arg in args {
        if arg == "--help" || arg == "-h" {
            route.help = true;
            route.consumed += 1;
            break;
        }
        if arg == "help" {
            route.help = true;
        } else if arg.as_encoded_bytes().starts_with(b"-") {
            break;
        } else if let Some(child) = route.command.find_subcommand(arg) {
            route.command = child;
            route.path.push(arg.clone());
        } else if route.path == ["test"] && !route.help {
            break;
        } else {
            let message = match (route.path.is_empty(), arg.to_str()) {
                (true, Some("create")) => "cargo nestrs create is not implemented yet; use cargo nestrs init to initialize an existing project. Run cargo nestrs init --help for usage".to_owned(),
                (true, Some("ide")) => "cargo nestrs ide has been renamed to cargo nestrs init; rerun init with your previous options to refresh generated editor commands".to_owned(),
                _ => format!("unrecognized subcommand {:?}", arg.to_string_lossy()),
            };
            return Err(route
                .command
                .clone()
                .error(ErrorKind::InvalidSubcommand, message));
        }
        route.consumed += 1;
    }
    Ok(route)
}

/// Cargo's required, separated value slots must not become Nestrs options.
/// This is an arity guard, not a Cargo validator: unknown options are preserved,
/// and optional values such as --timings=html do not consume the following token.
fn cargo_value_slot(arg: &std::ffi::OsStr) -> bool {
    matches!(
        arg.to_str(),
        Some(
            "--manifest-path"
                | "--target-dir"
                | "--config"
                | "--package"
                | "-p"
                | "--exclude"
                | "--bin"
                | "--example"
                | "--test"
                | "--bench"
                | "--target"
                | "--features"
                | "-F"
                | "--profile"
                | "--jobs"
                | "-j"
                | "--color"
                | "--message-format"
                | "--lockfile-path"
                | "--artifact-dir"
                | "--out-dir"
                | "-Z"
        )
    )
}

fn native_argument<'a>(command: &'a Command, token: &std::ffi::OsStr) -> Option<&'a Arg> {
    command.get_arguments().find(|arg| {
        arg.get_long().is_some_and(|long| {
            token == format!("--{long}").as_str()
                || token
                    .as_encoded_bytes()
                    .starts_with(format!("--{long}=").as_bytes())
        }) || arg
            .get_short()
            .is_some_and(|short| token == format!("-{short}").as_str())
    })
}

fn partition(command: &Command, args: &[OsString]) -> (Vec<OsString>, Vec<OsString>, bool) {
    if !command
        .get_arguments()
        .any(|arg| arg.get_id() == "cargo_args")
    {
        let help = super::before_separator(args)
            .iter()
            .any(|arg| arg == "--help" || arg == "-h");
        return (args.to_vec(), Vec::new(), help);
    }
    let mut native = Vec::new();
    let mut cargo = Vec::new();
    let mut args = args.iter().peekable();
    while let Some(token) = args.next() {
        if token == "--" {
            cargo.push(token.clone());
            cargo.extend(args.cloned());
            break;
        }
        if token == "--help" || token == "-h" {
            return (native, cargo, true);
        }
        if let Some(arg) = native_argument(command, token) {
            native.push(token.clone());
            if arg.get_action().takes_values()
                && !token.as_encoded_bytes().contains(&b'=')
                && args
                    .peek()
                    .is_some_and(|value| *value != "--" && *value != "--help" && *value != "-h")
            {
                native.push(args.next().unwrap().clone());
            }
        } else {
            cargo.push(token.clone());
            if cargo_value_slot(token) && args.peek().is_some_and(|value| *value != "--") {
                cargo.push(args.next().unwrap().clone());
            }
        }
    }
    (native, cargo, false)
}

pub(super) fn parse(mut args: Vec<OsString>) -> Result<Invocation, Error> {
    // Cargo plugins receive an extra `nestrs` word; direct invocations do not.
    if args.first().is_some_and(|arg| arg == "nestrs") {
        args.remove(0);
    }
    let mut root = command();
    root.build();
    let route = route(&root, &args)?;
    let (native, cargo, help) = partition(route.command, &args[route.consumed..]);
    let help = help || route.help;
    if help
        && route.path.len() == 1
        && let Some(&(name, _)) = CARGO_COMMANDS
            .iter()
            .find(|(name, _)| route.path[0] == *name)
    {
        return Ok(Invocation::CargoHelp(name.to_owned()));
    }
    let mut normalized = vec![OsString::from("cargo-nestrs")];
    normalized.extend(route.path.iter().cloned());
    if help {
        normalized.push("--help".into());
    } else {
        normalized.extend(native);
    }
    // Parse only native options. Keeping Cargo's payload separate avoids an
    // artificial `--` being mistaken for a missing native path value, and keeps
    // the user's original separator and OS strings completely intact.
    let matches = root.clone().try_get_matches_from(normalized)?;
    let (name, matches) = matches
        .subcommand()
        .expect("the root has no action without a subcommand");
    let check = name == "init" && matches.subcommand_name() == Some("check");
    let matches = if check {
        matches.subcommand_matches("check").unwrap()
    } else {
        matches
    };
    let cargo_args = cargo;
    match name {
        "doctor" => Ok(Invocation::Doctor),
        "graph" => Ok(Invocation::Graph(GraphOptions {
            output: matches.get_one::<PathBuf>("output").cloned(),
            cargo_args,
        })),
        "init" => {
            if check && matches.get_flag("vscode") {
                return Err(route.command.clone().error(ErrorKind::ArgumentConflict,
                    "init check does not modify editor settings; use cargo nestrs init --vscode for setup"));
            }
            Ok(Invocation::Init(InitOptions {
                check,
                vscode: matches.get_flag("vscode"),
                output: matches.get_one::<PathBuf>("output").cloned(),
                cargo_args,
            }))
        }
        _ => Ok(Invocation::Cargo {
            command: name.to_owned(),
            args: cargo_args,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn command_schema_is_consistent() {
        command().debug_assert();
    }

    #[test]
    fn init_options_are_typed_without_reordering_cargo_arguments() {
        for check in [false, true] {
            let mut args = arguments(&["init"]);
            if check {
                args.push("check".into());
            }
            args.extend(arguments(&[
                "--manifest-path",
                "project with spaces/Cargo.toml",
                "--features",
                "help",
                "--output=old.json",
                "--locked",
                "--output",
                "model directory/new.json",
            ]));
            assert_eq!(
                parse(args).unwrap(),
                Invocation::Init(InitOptions {
                    check,
                    vscode: false,
                    output: Some("model directory/new.json".into()),
                    cargo_args: arguments(&[
                        "--manifest-path",
                        "project with spaces/Cargo.toml",
                        "--features",
                        "help",
                        "--locked"
                    ]),
                })
            );
        }
        assert_eq!(
            parse(arguments(&["init", "--locked", "--vscode", "--vscode"])).unwrap(),
            Invocation::Init(InitOptions {
                check: false,
                vscode: true,
                output: None,
                cargo_args: arguments(&["--locked"]),
            })
        );
    }

    #[test]
    fn cargo_value_slots_cannot_become_nestrs_options_or_help() {
        let mut cargo = arguments(&[
            "--config",
            "--output=literal",
            "--bin",
            "help",
            "--features",
            "--vscode",
            "--manifest-path",
            "--help",
            "--future-option=unchanged",
        ]);
        let mut args = arguments(&["init"]);
        args.extend(cargo.clone());
        args.extend(arguments(&["--output", "real.json", "--offline"]));
        cargo.push("--offline".into());
        assert_eq!(
            parse(args).unwrap(),
            Invocation::Init(InitOptions {
                check: false,
                vscode: false,
                output: Some("real.json".into()),
                cargo_args: cargo,
            })
        );
    }

    #[test]
    fn dash_output_is_a_path_and_root_separator_reports_a_clap_error() {
        for path in ["-", "-report.html", "--vscode"] {
            assert_eq!(
                parse(arguments(&["graph", "--output", path])).unwrap(),
                Invocation::Graph(GraphOptions {
                    output: Some(path.into()),
                    cargo_args: Vec::new()
                })
            );
        }
        assert_eq!(
            parse(arguments(&["--"])).unwrap_err().kind(),
            ErrorKind::MissingSubcommand
        );
        assert_eq!(
            parse(arguments(&["doctor", "--unknown", "--help"]))
                .unwrap_err()
                .kind(),
            ErrorKind::DisplayHelp
        );
    }
}
