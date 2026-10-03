//! Initialize an existing Nestrs project's development environment.

use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    env,
    ffi::OsString,
    fs,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde_json::{Value, json};

use crate::{
    ide,
    toolchain::{Toolchain, cargo_program},
};

/// 检查真实编译单元，成功后刷新 IDE 模型；初次初始化可额外生成客户端设置。
pub(super) fn run(options: super::cli::InitOptions) -> Result<u8, String> {
    let super::cli::InitOptions {
        check: check_mode,
        vscode,
        output,
        mut cargo_args,
    } = options;
    if cargo_args.iter().any(|arg| {
        arg == "--"
            || arg
                .to_str()
                .is_some_and(|arg| arg.starts_with("--message-format"))
    }) {
        return Err("cargo nestrs init manages its own Cargo JSON output and does not accept program arguments".into());
    }
    let toolchain = Toolchain::discover()?;
    super::reject_wrappers(&toolchain)?;
    if let Some(target) = super::option_value(&cargo_args, "--target")?
        && target != toolchain.identity.host.as_str()
    {
        return Err("cargo nestrs init currently supports the pinned host target only".into());
    }
    let macro_server = toolchain.sysroot.join("libexec").join(format!(
        "rust-analyzer-proc-macro-srv{}",
        env::consts::EXE_SUFFIX
    ));
    if !macro_server.is_file() {
        return Err(format!(
            "the selected toolchain has no rust-analyzer proc-macro server: {}",
            macro_server.display()
        ));
    }
    if !toolchain
        .sysroot
        .join("lib/rustlib/src/rust/library/core/src/lib.rs")
        .is_file()
    {
        return Err("cargo nestrs init requires rust-src for the selected toolchain; no components were installed".into());
    }
    let target = super::target_directory(&cargo_args)?;
    let mut metadata_command = Command::new(cargo_program());
    metadata_command
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .args(super::metadata_options(&cargo_args)?);
    let metadata = metadata_command
        .output()
        .map_err(|error| error.to_string())?;
    if !metadata.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&metadata.stderr)
        ));
    }
    let metadata: Value =
        serde_json::from_slice(&metadata.stdout).map_err(|error| error.to_string())?;
    let workspace = PathBuf::from(
        metadata["workspace_root"]
            .as_str()
            .ok_or("missing workspace_root")?,
    )
    .canonicalize()
    .map_err(|error| format!("cannot resolve Cargo workspace: {error}"))?;
    let selected_package = cargo_args.iter().any(|arg| selects_package(arg));
    if !selected_package {
        cargo_args.push("--workspace".into());
    }
    let selected_target = cargo_args.iter().any(|arg| {
        arg.to_str().is_some_and(|arg| {
            [
                "--lib",
                "--bin",
                "--bins",
                "--test",
                "--tests",
                "--example",
                "--examples",
                "--bench",
                "--benches",
                "--all-targets",
            ]
            .iter()
            .any(|flag| arg == *flag || arg.starts_with(&format!("{flag}=")))
        })
    });
    if !selected_target {
        cargo_args.push("--all-targets".into());
    }
    // Normalize location options once so setup and check-on-save reuse the same
    // compilation cache, even when launched from different working directories.
    let manifest = super::option_value(&cargo_args, "--manifest-path")?
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("Cargo.toml"))
        .canonicalize()
        .map_err(|error| error.to_string())?;
    cargo_args = normalize_locations(
        cargo_args,
        manifest,
        &env::current_dir().map_err(|error| error.to_string())?,
    )?;
    let mut hash = DefaultHasher::new();
    cargo_args.hash(&mut hash);
    workspace.hash(&mut hash);
    let build = toolchain
        .cache_directory(&target)
        .join("ide")
        .join(format!("{:016x}", hash.finish()));
    let captures = build.join("nestrs/ide-units");
    fs::create_dir_all(&captures).map_err(|error| error.to_string())?;
    let args = super::replace_target_directory(cargo_args.clone(), &build)?;
    let mut child = Command::new(cargo_program());
    child.arg("check").args(args).arg("--message-format=json");
    toolchain.configure(&mut child)?;
    child
        .env("NESTRS_COMPILER_OUTPUT", build.join("nestrs/compiler"))
        .env("NESTRS_IDE_CAPTURE", &captures)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = child
        .spawn()
        .map_err(|error| format!("cannot start Cargo: {error}"))?;
    let mut messages = Vec::new();
    let read_result = (|| -> Result<(), String> {
        for line in BufReader::new(child.stdout.take().ok_or("missing Cargo output")?).lines() {
            let line = line.map_err(|error| error.to_string())?;
            let message: Value = serde_json::from_str(&line)
                .map_err(|error| format!("invalid Cargo JSON message: {error}"))?;
            if check_mode {
                println!("{line}");
            } else if message["reason"] == "compiler-message"
                && let Some(rendered) = message["message"]["rendered"].as_str()
            {
                eprint!("{rendered}");
            }
            messages.push(message);
        }
        Ok(())
    })();
    let status = child.wait().map_err(|error| error.to_string())?;
    read_result?;
    if !status.success() {
        return Err(
            "Nestrs project preparation failed; the previous editor project and settings were preserved"
                .into(),
        );
    }
    let destination = output.unwrap_or_else(|| target.join("nestrs/ide/rust-project.json"));
    let destination = if destination.is_absolute() {
        destination
    } else {
        env::current_dir()
            .map_err(|error| error.to_string())?
            .join(destination)
    };
    let project = ide::generate(
        &captures,
        &messages,
        &metadata,
        &toolchain,
        &target,
        &destination,
    )?;
    ide::write_atomic(
        &destination,
        &serde_json::to_vec_pretty(&project).map_err(|error| error.to_string())?,
    )?;
    if check_mode {
        return Ok(0);
    }
    let executable = env::current_exe().map_err(|error| error.to_string())?;
    let mut check = vec![
        executable.to_string_lossy().into_owned(),
        "init".into(),
        "check".into(),
        "--output".into(),
        destination.to_string_lossy().into_owned(),
        "--target-dir".into(),
        target.to_string_lossy().into_owned(),
    ];
    for arg in &cargo_args {
        check.push(
            arg.to_str()
                .ok_or("editor command arguments must be UTF-8")?
                .to_owned(),
        );
    }
    let check_env: BTreeMap<_, _> = [
        ("NESTRS_RUSTC", &toolchain.rustc),
        ("NESTRS_DRIVER", &toolchain.driver),
        ("NESTRS_MACRO_BRIDGE", &toolchain.bridge),
    ]
    .into_iter()
    .map(|(name, path)| {
        path.to_str()
            .map(|value| (name.to_owned(), value.to_owned()))
            .ok_or("editor tool paths must be UTF-8")
    })
    .collect::<Result<_, _>>()?;
    let suggested = json!({
        "rust-analyzer.linkedProjects": [destination],
        "rust-analyzer.procMacro.enable": true,
        "rust-analyzer.procMacro.server": macro_server,
        "rust-analyzer.check.overrideCommand": check,
        "rust-analyzer.check.extraEnv": check_env,
        "rust-analyzer.checkOnSave": true,
        "rust-analyzer.cfg.setTest": false,
        "rust-analyzer.cargo.cfgs": [],
    });
    let settings_path = destination.with_file_name("rust-analyzer-settings.json");
    ide::write_atomic(
        &settings_path,
        &serde_json::to_vec_pretty(&suggested).map_err(|error| error.to_string())?,
    )?;
    if vscode {
        let settings = ide::configure(&workspace, &destination, check, &check_env, &macro_server)?;
        println!("Editor settings: {}", settings.display());
    }
    println!(
        "Initialized Nestrs development environment: {}",
        workspace.display()
    );
    println!("Rust analyzer project: {}", destination.display());
    println!("Editor configuration: {}", settings_path.display());
    if !vscode {
        println!(
            "Load the generated settings in your rust-analyzer client; use cargo nestrs init --vscode to configure VS Code."
        );
    }
    println!(
        "Prepared {} compiler units and the private declaration bridge",
        project["crates"]
            .as_array()
            .map_or(0, |crates| crates.len().saturating_sub(1))
    );
    Ok(0)
}

/// 判断用户是否已经选择 package 或 workspace，避免追加覆盖性默认选择。
fn selects_package(arg: &std::ffi::OsStr) -> bool {
    arg == "--package"
        || arg == "--workspace"
        || arg
            .to_str()
            .is_some_and(|arg| arg.starts_with("-p") || arg.starts_with("--package="))
}

/// 将 manifest 和配置文件路径固定到当前目录，保证初始化与保存检查复用缓存。
fn normalize_locations(
    args: Vec<OsString>,
    manifest: PathBuf,
    cwd: &Path,
) -> Result<Vec<OsString>, String> {
    let config = |value: OsString| -> Result<OsString, String> {
        if value.to_str().is_some_and(|value| value.contains('=')) {
            // Inline TOML is intentionally retained as one Cargo argument.
            Ok(value)
        } else {
            cwd.join(value)
                .canonicalize()
                .map(PathBuf::into_os_string)
                .map_err(|error| format!("cannot resolve Cargo --config file: {error}"))
        }
    };
    let mut normalized = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--manifest-path" || arg == "--target-dir" {
            iter.next();
        } else if arg == "--config" {
            let value = iter.next().ok_or("--config requires a value")?;
            normalized.extend([arg, config(value)?]);
        } else if let Some(value) = arg.to_str().and_then(|arg| arg.strip_prefix("--config=")) {
            normalized.extend(["--config".into(), config(value.into())?]);
        } else if !arg.to_str().is_some_and(|arg| {
            arg.starts_with("--manifest-path=") || arg.starts_with("--target-dir=")
        }) {
            normalized.push(arg);
        }
    }
    normalized.extend(["--manifest-path".into(), manifest.into_os_string()]);
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_time_options_preserve_package_and_config_selection() {
        for option in ["-p", "-papp", "-p=app", "--package=app", "--workspace"] {
            assert!(selects_package(option.as_ref()));
        }
        assert!(!selects_package("--profile=dev".as_ref()));
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let args = normalize_locations(
            ["-papp", "--config=./Cargo.toml", "--config", "build.jobs=2"]
                .map(OsString::from)
                .to_vec(),
            root.join("Cargo.toml"),
            root,
        )
        .unwrap();
        assert_eq!(
            args,
            [
                "-papp".into(),
                "--config".into(),
                root.join("Cargo.toml")
                    .canonicalize()
                    .unwrap()
                    .into_os_string(),
                "--config".into(),
                "build.jobs=2".into(),
                "--manifest-path".into(),
                root.join("Cargo.toml").into_os_string(),
            ]
        );
    }
}
