//! Cargo-facing entry points. Application arguments are kept as OS strings.

use std::{
    env,
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    process::{Command, ExitCode, ExitStatus},
};

use crate::toolchain::{Toolchain, cargo_program};

mod graph;
mod help;
mod init;

#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Help(help::Topic),
    Version,
    Doctor,
    Graph(Vec<OsString>),
    Init(Vec<OsString>),
    Cargo {
        command: String,
        args: Vec<OsString>,
    },
}

pub fn run() -> ExitCode {
    match execute(env::args_os().skip(1).collect()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn execute(args: Vec<OsString>) -> Result<u8, String> {
    match parse(args)? {
        Invocation::Help(topic) => help::show(topic),
        Invocation::Version => {
            println!("cargo-nestrs {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Invocation::Doctor => {
            let toolchain = Toolchain::discover()?;
            println!("Nestrs toolchain is ready");
            println!(
                "rustc: {} ({})",
                toolchain.identity.release, toolchain.identity.commit
            );
            println!("host: {}", toolchain.identity.host);
            println!("compiler: {}", toolchain.rustc.display());
            println!("sysroot: {}", toolchain.sysroot.display());
            println!("driver: {}", toolchain.driver.display());
            println!("macro bridge: {}", toolchain.bridge.display());
            println!("driver fingerprint: {}", toolchain.fingerprint);
            Ok(0)
        }
        Invocation::Cargo { command, args } => run_cargo(&command, args),
        Invocation::Graph(args) => graph::run(args),
        Invocation::Init(args) => init::run(args),
    }
}

fn parse(mut args: Vec<OsString>) -> Result<Invocation, String> {
    // Cargo calls `cargo-nestrs nestrs ...`; direct invocations omit this word.
    if args.first().is_some_and(|arg| arg == "nestrs") {
        args.remove(0);
    }
    // Resolve help before discovering tools or doing any project work.
    if let Some(topic) = help::resolve(&args)? {
        return Ok(Invocation::Help(topic));
    }
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err("command must be valid UTF-8".into());
    };
    match command {
        "-V" | "--version" => Ok(Invocation::Version),
        "doctor" if args.len() == 1 => Ok(Invocation::Doctor),
        "doctor" => Err("doctor does not take Cargo arguments".into()),
        "graph" => {
            args.remove(0);
            Ok(Invocation::Graph(args))
        }
        "init" => {
            args.remove(0);
            Ok(Invocation::Init(args))
        }
        "ide" => Err("cargo nestrs ide has been renamed to cargo nestrs init; rerun init with your previous options to refresh generated editor commands".into()),
        "check" | "build" | "run" | "test" => {
            let command = command.to_owned();
            args.remove(0);
            Ok(Invocation::Cargo { command, args })
        }
        unknown => Err(format!(
            "unknown Nestrs command {unknown:?}; run cargo nestrs --help"
        )),
    }
}

fn run_cargo(command: &str, args: Vec<OsString>) -> Result<u8, String> {
    let toolchain = Toolchain::discover()?;
    reject_wrappers(&toolchain)?;
    let target = target_directory(&args)?;
    let isolated = toolchain.cache_directory(&target);
    let args = replace_target_directory(args, &isolated)?;
    let mut child = Command::new(cargo_program());
    child.arg(command).args(args);
    toolchain.configure(&mut child)?;
    child.env("CARGO_TARGET_DIR", &isolated).env(
        "NESTRS_COMPILER_OUTPUT",
        isolated.join("nestrs").join("compiler"),
    );
    spawn(&mut child)
}

pub(super) fn reject_wrappers(toolchain: &Toolchain) -> Result<(), String> {
    for variable in ["RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER"] {
        if let Some(value) = env::var_os(variable)
            && !value.is_empty()
            && !(variable == "RUSTC_WRAPPER"
                && Path::new(&value).canonicalize().ok().as_ref() == Some(&toolchain.driver))
        {
            return Err(format!(
                "{variable} is already configured; composing other compiler wrappers is not supported. Unset {variable} for cargo nestrs",
            ));
        }
    }
    Ok(())
}

pub(super) fn before_separator(args: &[OsString]) -> &[OsString] {
    let end = args
        .iter()
        .position(|arg| arg == "--")
        .unwrap_or(args.len());
    &args[..end]
}

pub(super) fn option_value(args: &[OsString], name: &str) -> Result<Option<OsString>, String> {
    let args = before_separator(args);
    let mut found = None;
    let mut position = 0;
    while position < args.len() {
        if args[position] == name {
            position += 1;
            let value = args
                .get(position)
                .filter(|arg| !arg.is_empty())
                .ok_or_else(|| format!("{name} requires a value"))?;
            found = Some(value.clone());
        } else if let Some(value) = option_assignment(&args[position], name) {
            if value.is_empty() {
                return Err(format!("{name} requires a value"));
            }
            found = Some(value);
        }
        position += 1;
    }
    Ok(found)
}

fn option_assignment(arg: &OsStr, name: &str) -> Option<OsString> {
    let prefix = format!("{name}=");
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        arg.as_bytes()
            .strip_prefix(prefix.as_bytes())
            .map(OsStr::from_bytes)
            .map(OsStr::to_os_string)
    }
    #[cfg(not(unix))]
    {
        arg.to_str()
            .and_then(|arg| arg.strip_prefix(&prefix))
            .map(OsString::from)
    }
}

pub(super) fn target_directory(args: &[OsString]) -> Result<PathBuf, String> {
    let configured = option_value(args, "--target-dir")?
        .or_else(|| env::var_os("CARGO_TARGET_DIR").filter(|path| !path.is_empty()));
    let directory = if let Some(directory) = configured {
        PathBuf::from(directory)
    } else {
        // Cargo resolves workspace roots and .cargo/config.toml target-dir. It
        // remains the source of truth; guessing from cwd breaks member crates.
        let mut metadata = Command::new(cargo_program());
        metadata.args(["metadata", "--no-deps", "--format-version", "1"]);
        metadata.args(metadata_options(args)?);
        let output = metadata
            .output()
            .map_err(|error| format!("cannot run cargo metadata: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "cargo metadata failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let json: serde_json::Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("cannot parse cargo metadata: {error}"))?;
        PathBuf::from(
            json.get("target_directory")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "cargo metadata did not return target_directory".to_owned())?,
        )
    };
    if directory.is_absolute() {
        Ok(directory)
    } else {
        env::current_dir()
            .map(|cwd| cwd.join(directory))
            .map_err(|error| format!("cannot resolve target directory: {error}"))
    }
}

pub(super) fn metadata_options(args: &[OsString]) -> Result<Vec<OsString>, String> {
    let mut forwarded = Vec::new();
    let mut args = before_separator(args).iter();
    while let Some(arg) = args.next() {
        if arg == "--manifest-path" || arg == "--config" {
            let value = args
                .next()
                .ok_or_else(|| format!("{} requires a value", arg.to_string_lossy()))?;
            forwarded.extend([arg.clone(), value.clone()]);
        } else if arg == "--offline"
            || arg == "--locked"
            || arg == "--frozen"
            || option_assignment(arg, "--manifest-path").is_some()
            || option_assignment(arg, "--config").is_some()
        {
            forwarded.push(arg.clone());
        }
    }
    Ok(forwarded)
}

pub(super) fn replace_target_directory(
    args: Vec<OsString>,
    isolated: &Path,
) -> Result<Vec<OsString>, String> {
    let mut result = Vec::with_capacity(args.len() + 2);
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            result.extend([
                OsString::from("--target-dir"),
                isolated.as_os_str().to_owned(),
                arg,
            ]);
            result.extend(args);
            return Ok(result);
        }
        if arg == "--target-dir" {
            args.next()
                .filter(|arg| arg != "--")
                .ok_or_else(|| "--target-dir requires a value".to_owned())?;
        } else if option_assignment(&arg, "--target-dir").is_none() {
            result.push(arg);
        }
    }
    result.extend([
        OsString::from("--target-dir"),
        isolated.as_os_str().to_owned(),
    ]);
    Ok(result)
}

fn spawn(command: &mut Command) -> Result<u8, String> {
    command.status().map(exit_code).map_err(|error| {
        format!(
            "cannot execute {}: {error}",
            command.get_program().to_string_lossy()
        )
    })
}

pub(super) fn exit_code(status: ExitStatus) -> u8 {
    if let Some(code) = status.code() {
        return code as u8;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return (128 + signal) as u8;
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn cargo_plugin_and_direct_forms_are_identical() {
        assert_eq!(
            parse(arguments(&["nestrs", "build", "--locked"])).unwrap(),
            parse(arguments(&["build", "--locked"])).unwrap()
        );
        assert_eq!(
            parse(Vec::new()).unwrap(),
            Invocation::Help(help::Topic::Root)
        );
        assert!(parse(arguments(&["install"])).is_err());
    }

    #[test]
    fn init_routes_cargo_options_without_reinterpreting_them() {
        let args = [
            "--manifest-path",
            "app/Cargo.toml",
            "--features",
            "server",
            "--vscode",
        ];
        let mut command = vec![OsString::from("nestrs"), OsString::from("init")];
        command.extend(arguments(&args));
        assert_eq!(
            parse(command.clone()).unwrap(),
            Invocation::Init(arguments(&args))
        );
        command.remove(0);
        assert_eq!(parse(command).unwrap(), Invocation::Init(arguments(&args)));
        assert_eq!(
            parse(arguments(&["init", "check", "--locked"])).unwrap(),
            Invocation::Init(arguments(&["check", "--locked"]))
        );
        assert!(
            parse(arguments(&["ide"]))
                .unwrap_err()
                .contains("renamed to cargo nestrs init")
        );
    }

    #[test]
    fn application_options_are_never_parsed_as_cargo_options() {
        let args = arguments(&[
            "--features",
            "a,b",
            "--target-dir=custom",
            "--",
            "--target-dir",
            "app-data",
            "two words",
        ]);
        assert_eq!(
            option_value(&args, "--target-dir").unwrap(),
            Some("custom".into())
        );
        let replaced = replace_target_directory(args, Path::new("isolated")).unwrap();
        assert_eq!(
            replaced,
            arguments(&[
                "--features",
                "a,b",
                "--target-dir",
                "isolated",
                "--",
                "--target-dir",
                "app-data",
                "two words"
            ])
        );
    }

    #[test]
    fn cargo_metadata_preserves_configuration_and_network_constraints() {
        let args = arguments(&[
            "--manifest-path",
            "member/Cargo.toml",
            "--config=build.target-dir='custom'",
            "--offline",
            "--locked",
            "--features",
            "a",
            "--",
            "--frozen",
        ]);
        assert_eq!(
            metadata_options(&args).unwrap(),
            arguments(&[
                "--manifest-path",
                "member/Cargo.toml",
                "--config=build.target-dir='custom'",
                "--offline",
                "--locked"
            ])
        );
    }

    #[test]
    fn malformed_target_option_is_rejected() {
        assert!(option_value(&arguments(&["--target-dir"]), "--target-dir").is_err());
        assert!(option_value(&arguments(&["--target-dir", "--", "app"]), "--target-dir").is_err());
        assert!(option_value(&arguments(&["--target-dir="]), "--target-dir").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_application_arguments_are_preserved() {
        use std::os::unix::ffi::OsStringExt;
        let bytes = OsString::from_vec(vec![0xff, 0xfe]);
        let args = vec!["--".into(), bytes.clone()];
        let forwarded = replace_target_directory(args, Path::new("target")).unwrap();
        assert_eq!(forwarded.last(), Some(&bytes));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_target_path_is_resolved_without_loss() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![0xff, b'/', b't']);
        let mut option = OsString::from("--target-dir=");
        option.push(&path);
        assert_eq!(
            option_value(&[option.clone()], "--target-dir").unwrap(),
            Some(path)
        );
        assert_eq!(
            replace_target_directory(vec![option], Path::new("isolated")).unwrap(),
            arguments(&["--target-dir", "isolated"])
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_exit_status_and_signals_are_preserved() {
        assert_eq!(
            spawn(Command::new("sh").args(["-c", "exit 73"])).unwrap(),
            73
        );
        assert_eq!(
            spawn(Command::new("sh").args(["-c", "kill -TERM $$"])).unwrap(),
            143
        );
    }
}
