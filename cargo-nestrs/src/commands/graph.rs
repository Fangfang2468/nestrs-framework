use std::{
    ffi::OsString,
    fs,
    hash::{Hash, Hasher},
    path::PathBuf,
    process::Command,
};

use super::{
    before_separator, metadata_options, reject_wrappers, replace_target_directory, target_directory,
};
use crate::toolchain::{
    Toolchain, cargo_program, library_path_variable, runtime_library_directories,
};

pub(super) const HELP: &str =
    "Usage: cargo nestrs graph [CARGO BUILD OPTIONS] [--bin NAME] [--output PATH]

Without --bin, export one project report containing every binary in the selected
package (or --workspace). Each entry is built and validated independently. Entries
requiring disabled features are reported as skipped. Errors remain visible in the
report and return a nonzero exit status; valid entries are retained.
Use -p PACKAGE --features FEATURES for feature selection across multiple packages;
--workspace --features is not supported. --all-features applies per package.

With --bin, export only that binary's validated graph. Failure preserves any previous
output. Both modes avoid the business main and service constructors.
Default output: <Cargo target directory>/nestrs-di.html

Options:
  -p, --package PACKAGE  Select a package
  --workspace           Include all workspace packages with binary entries
  --bin NAME            Limit the report to one binary
  --output PATH         Write HTML to this file
  -h, --help            Show this help

Equivalent help: cargo nestrs help graph / cargo nestrs graph help
";

#[derive(Clone, Debug)]
struct GraphTarget {
    package_id: String,
    package: String,
    binary: String,
    source: PathBuf,
    manifest_dir: PathBuf,
    required_features: Vec<String>,
}

struct Selection {
    packages: Vec<serde_json::Value>,
    targets: Vec<GraphTarget>,
}

#[derive(Debug)]
struct EntryFailure {
    diagnostic: String,
    skipped: bool,
}

impl From<String> for EntryFailure {
    fn from(diagnostic: String) -> Self {
        Self {
            diagnostic,
            skipped: false,
        }
    }
}

pub(super) fn run(args: Vec<OsString>) -> Result<u8, String> {
    let (args, output) = extract_output(args)?;
    if before_separator(&args).len() != args.len() {
        return Err("graph does not run the application and takes no arguments after --".into());
    }
    validate_selectors(&args)?;
    let toolchain = Toolchain::discover()?;
    reject_wrappers(&toolchain)?;
    let requested_target =
        super::option_value(&args, "--target")?.or_else(|| std::env::var_os("CARGO_BUILD_TARGET"));
    if requested_target
        .as_ref()
        .is_some_and(|target| target != toolchain.identity.host.as_str())
    {
        return Err("graph requires a target executable on the pinned host; cross-target graph execution is not supported".into());
    }
    let target = target_directory(&args)?;
    let metadata = Command::new(cargo_program())
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .args(metadata_options(&args)?)
        .output()
        .map_err(|error| format!("cannot query graph targets: {error}"))?;
    if !metadata.status.success() {
        return Err(String::from_utf8_lossy(&metadata.stderr).into_owned());
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).map_err(|error| error.to_string())?;
    let selected = select_targets(&metadata, &args)?;
    let single = super::option_value(&args, "--bin")?.is_some();
    let build_args = per_entry_arguments(&args)?;
    let mut errors = 0;
    let data = if single {
        inspect_target(&toolchain, &target, &build_args, &selected.targets[0])
            .map_err(|failure| failure.diagnostic)?
    } else {
        let mut entries = Vec::new();
        for entry in &selected.targets {
            eprintln!("Graph entry: {} / {}", entry.package, entry.binary);
            let (status, graph, diagnostic) =
                match inspect_target(&toolchain, &target, &build_args, entry) {
                    Ok(graph) => ("ok", Some(graph), None),
                    Err(failure) => {
                        let status = if failure.skipped {
                            "skipped"
                        } else {
                            errors += 1;
                            "error"
                        };
                        eprintln!(
                            "{} / {}: {status}: {}",
                            entry.package, entry.binary, failure.diagnostic
                        );
                        (status, None, Some(failure.diagnostic))
                    }
                };
            entries.push(serde_json::json!({
                "id": format!("{}::{}", entry.package_id, entry.binary),
                "packageId": entry.package_id,
                "package": entry.package,
                "binary": entry.binary,
                "status": status,
                "graph": graph,
                "diagnostic": diagnostic,
                "requiredFeatures": entry.required_features,
            }));
        }
        serde_json::json!({"version": 2, "kind": "project", "packages": selected.packages, "entries": entries})
    };
    let html = crate::graph::render_html(&data)?;
    let output = export_html(
        output.unwrap_or_else(|| target.join("nestrs-di.html")),
        html,
    )?;
    if single {
        println!("DI graph: {}", output.display());
    } else {
        let entries = data["entries"].as_array().expect("project entries");
        let valid = entries
            .iter()
            .filter(|entry| entry["status"] == "ok")
            .count();
        let skipped = entries.len() - valid - errors;
        println!(
            "DI project graph: {} ({valid} valid, {errors} errors, {skipped} skipped)",
            output.display()
        );
    }
    Ok(u8::from(errors != 0))
}

fn select_targets(metadata: &serde_json::Value, args: &[OsString]) -> Result<Selection, String> {
    let package = super::option_value(args, "--package")?.or(super::option_value(args, "-p")?);
    let binary = super::option_value(args, "--bin")?;
    let all = args.iter().any(|arg| arg == "--workspace");
    let defaults = metadata["workspace_default_members"].as_array();
    let members = metadata["workspace_members"]
        .as_array()
        .ok_or("Cargo metadata has no workspace members")?;
    let mut selection = Selection {
        packages: Vec::new(),
        targets: Vec::new(),
    };
    for entry in metadata["packages"]
        .as_array()
        .ok_or("Cargo metadata has no packages")?
    {
        let id = entry["id"].as_str().ok_or("package has no id")?;
        let name = entry["name"].as_str().ok_or("package has no name")?;
        let version = entry["version"].as_str().ok_or("package has no version")?;
        if !members.iter().any(|member| member == id) {
            continue;
        }
        let selected = match &package {
            Some(package) => {
                package == name || package == id || package == format!("{name}@{version}").as_str()
            }
            None => {
                all || defaults.is_some_and(|members| members.iter().any(|member| member == id))
            }
        };
        if !selected {
            continue;
        }
        selection
            .packages
            .push(serde_json::json!({"id": id, "name": name, "version": version}));
        let manifest = PathBuf::from(
            entry["manifest_path"]
                .as_str()
                .ok_or("package has no manifest_path")?,
        );
        let manifest_dir = manifest
            .parent()
            .ok_or("package manifest has no directory")?
            .to_owned();
        for target in entry["targets"]
            .as_array()
            .ok_or("package has no targets")?
        {
            if !target["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
            {
                continue;
            }
            let name = target["name"].as_str().ok_or("target has no name")?;
            if binary.as_ref().is_some_and(|wanted| wanted != name) {
                continue;
            }
            let required_features = target["required-features"]
                .as_array()
                .map(|features| {
                    features
                        .iter()
                        .map(|feature| {
                            feature
                                .as_str()
                                .map(str::to_owned)
                                .ok_or("invalid required feature".to_owned())
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .transpose()?
                .unwrap_or_default();
            selection.targets.push(GraphTarget {
                package_id: id.to_owned(),
                package: entry["name"].as_str().unwrap().to_owned(),
                binary: name.to_owned(),
                source: PathBuf::from(
                    target["src_path"]
                        .as_str()
                        .ok_or("target has no src_path")?,
                ),
                manifest_dir: manifest_dir.clone(),
                required_features,
            });
        }
    }
    selection.packages.sort_by(|a, b| {
        a["name"]
            .as_str()
            .cmp(&b["name"].as_str())
            .then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
    });
    selection.targets.sort_by(|a, b| {
        (&a.package, &a.binary, &a.package_id).cmp(&(&b.package, &b.binary, &b.package_id))
    });
    if selection.targets.is_empty() {
        return Err("graph found no binary targets in the selected project; select a package with a binary using -p PACKAGE (library/test/example graphs are not supported)".into());
    }
    if binary.is_some() && selection.targets.len() != 1 {
        return Err(format!(
            "graph --bin requires exactly one binary; select -p PACKAGE to disambiguate (found {})",
            selection.targets.len()
        ));
    }
    if selection.packages.len() > 1 && has_feature_selection(args) {
        return Err(FEATURE_SELECTION_ERROR.into());
    }
    Ok(selection)
}

/// Select exactly one build unit even when the caller selected a workspace.
/// Feature/profile/config arguments retain their original values and order.
fn per_entry_arguments(args: &[OsString]) -> Result<Vec<OsString>, String> {
    let mut retained = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if ["-p", "--package", "--bin", "--target"]
            .iter()
            .any(|flag| arg == *flag)
        {
            args.next()
                .ok_or_else(|| format!("{} requires a value", arg.to_string_lossy()))?;
        } else if arg != "--workspace"
            && !arg.to_str().is_some_and(|arg| {
                ["--package=", "--bin=", "--target="]
                    .iter()
                    .any(|flag| arg.starts_with(flag))
            })
        {
            retained.push(arg.clone());
        }
    }
    Ok(retained)
}

fn inspect_target(
    toolchain: &Toolchain,
    target: &std::path::Path,
    args: &[OsString],
    entry: &GraphTarget,
) -> Result<serde_json::Value, EntryFailure> {
    let mut identity = std::collections::hash_map::DefaultHasher::new();
    (&entry.package_id, &entry.binary).hash(&mut identity);
    let isolated = if cfg!(windows) {
        // MSVC also limits paths of build-script outputs. Hash the complete
        // compiler/tool/entry identity into one short graph directory.
        toolchain.identity.cache_key().hash(&mut identity);
        toolchain.fingerprint.hash(&mut identity);
        target
            .join("nestrs")
            .join(format!("g{:016x}", identity.finish()))
    } else {
        toolchain
            .cache_directory(target)
            .join("graph")
            .join(format!("{}-{:016x}", entry.binary, identity.finish()))
    };
    let manifest_dir = entry
        .manifest_dir
        .canonicalize()
        .map_err(|error| format!("cannot resolve graph package: {error}"))?;
    let proof_path = isolated.join("graph-entry.json");
    let mut build = Command::new(cargo_program());
    build
        .arg("build")
        .args(replace_target_directory(args.to_vec(), &isolated)?)
        .args([
            "--package",
            &entry.package_id,
            "--bin",
            &entry.binary,
            "--message-format=json",
            "--target",
            &toolchain.identity.host,
        ]);
    toolchain.configure(&mut build)?;
    build
        .env("NESTRS_GRAPH_TARGET", entry.binary.replace('-', "_"))
        .env("NESTRS_GRAPH_BINARY", &entry.binary)
        .env("NESTRS_GRAPH_MANIFEST", &manifest_dir)
        .env("NESTRS_GRAPH_SOURCE", &entry.source)
        .env("NESTRS_GRAPH_PROOF", &proof_path)
        .env_remove("NESTRS_IDE_CAPTURE")
        .env("NESTRS_COMPILER_OUTPUT", isolated.join("compiler"));
    let built = build
        .output()
        .map_err(|error| format!("cannot build graph target: {error}"))?;
    let stderr = plain_diagnostic(&String::from_utf8_lossy(&built.stderr));
    // Cargo owns feature closure and required-features semantics. Do not guess
    // from --features text: aliases, defaults and dependency features matter.
    if !built.status.success()
        && !entry.required_features.is_empty()
        && stderr.lines().any(|line| {
            line.starts_with(&format!("error: target `{}` ", entry.binary))
                && line.contains(" requires the features:")
        })
    {
        return Err(EntryFailure {
            skipped: true,
            diagnostic: format!(
                "disabled required features: {}; enable them with --features",
                entry.required_features.join(", ")
            ),
        });
    }
    eprint!("{stderr}");
    let mut executable = None;
    let mut diagnostics = Vec::new();
    for line in built
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let Ok(message) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        if message["reason"] == "compiler-message"
            && let Some(rendered) = message["message"]["rendered"].as_str()
        {
            eprint!("{rendered}");
            if message["message"]["level"] == "error" {
                diagnostics.push(plain_diagnostic(rendered));
            }
        }
        if message["reason"] == "compiler-artifact"
            && message["package_id"] == entry.package_id
            && message["target"]["name"] == entry.binary
            && message["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
            && message["target"]["src_path"]
                .as_str()
                .is_some_and(|path| same_path(std::path::Path::new(path), &entry.source))
            && let Some(path) = message["executable"].as_str()
            && executable.replace(PathBuf::from(path)).is_some()
        {
            return Err("Cargo produced multiple graph executables"
                .to_owned()
                .into());
        }
    }
    if !built.status.success() {
        if diagnostics.is_empty() {
            diagnostics.extend(
                stderr
                    .lines()
                    .filter(|line| {
                        !line.trim().is_empty()
                            && ![
                                "Compiling ",
                                "Checking ",
                                "Finished ",
                                "Blocking ",
                                "Waiting ",
                            ]
                            .iter()
                            .any(|prefix| line.trim_start().starts_with(prefix))
                    })
                    .map(str::to_owned),
            );
        }
        return Err(format!(
            "cannot build graph entry {} / {}:\n{}",
            entry.package,
            entry.binary,
            diagnostics.join("\n")
        )
        .into());
    }
    let executable = executable.ok_or_else(|| "Cargo produced no graph executable".to_owned())?;
    let proof: serde_json::Value =
        serde_json::from_slice(&fs::read(&proof_path).map_err(|error| {
            format!(
                "missing graph entry verification; refusing to execute {}: {error}",
                executable.display()
            )
        })?)
        .map_err(|error| format!("invalid graph entry verification: {error}"))?;
    if proof["binary"] != entry.binary
        || proof["crate"] != entry.binary.replace('-', "_")
        || !proof["manifest"]
            .as_str()
            .is_some_and(|path| same_path(std::path::Path::new(path), &manifest_dir))
        || !proof["source"]
            .as_str()
            .is_some_and(|path| same_path(std::path::Path::new(path), &entry.source))
    {
        return Err(
            "graph entry verification does not match the selected binary; refusing to execute it"
                .to_owned()
                .into(),
        );
    }
    let mut inspect = Command::new(&executable);
    let mut library_paths =
        runtime_library_directories(&toolchain.sysroot, &toolchain.identity.host);
    library_paths.push(
        toolchain
            .sysroot
            .join("lib/rustlib")
            .join(&toolchain.identity.host)
            .join("lib"),
    );
    if let Some(profile) = executable.parent() {
        library_paths.extend([profile.to_owned(), profile.join("deps")]);
    }
    let library_variable = library_path_variable();
    if let Some(current) = std::env::var_os(library_variable) {
        library_paths.extend(std::env::split_paths(&current));
    }
    inspect.env(
        library_variable,
        std::env::join_paths(library_paths).map_err(|error| error.to_string())?,
    );
    let graph = inspect
        .output()
        .map_err(|error| format!("cannot inspect graph: {error}"))?;
    if !graph.status.success() {
        return Err(format!(
            "graph validation failed: {}",
            String::from_utf8_lossy(&graph.stderr).trim()
        )
        .into());
    }
    let data: serde_json::Value = serde_json::from_slice(&graph.stdout)
        .map_err(|error| format!("invalid graph response: {error}"))?;
    if data["version"] != 1 || !data["nodes"].is_array() {
        return Err("unsupported graph response".to_owned().into());
    }
    Ok(data)
}

fn same_path(left: &std::path::Path, right: &std::path::Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// Cargo can force colored diagnostics in CI. Preserve their text in the HTML
/// and in required-features detection without terminal escape sequences.
fn plain_diagnostic(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            plain.push(character);
            continue;
        }
        match characters.next() {
            Some('[') => {
                for code in characters.by_ref() {
                    if ('@'..='~').contains(&code) {
                        break;
                    }
                }
            }
            Some(']') => {
                while let Some(code) = characters.next() {
                    if code == '\u{7}' {
                        break;
                    }
                    if code == '\u{1b}' && characters.peek() == Some(&'\\') {
                        characters.next();
                        break;
                    }
                }
            }
            Some(other) => plain.push(other),
            None => {}
        }
    }
    plain
}

fn export_html(output: PathBuf, html: String) -> Result<PathBuf, String> {
    let output = if output.is_absolute() {
        output
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(output)
    };
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let temporary = output.with_extension(format!("html.{}.tmp", std::process::id()));
    let written = fs::write(&temporary, html).and_then(|()| fs::rename(&temporary, &output));
    if let Err(error) = written {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot export graph {}: {error}", output.display()));
    }
    Ok(output)
}

const FEATURE_SELECTION_ERROR: &str = "workspace graph entries are compiled independently; select -p PACKAGE --features FEATURES for package-specific features, or use --workspace --all-features";

fn has_feature_selection(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        arg == "--features"
            || arg == "-F"
            || arg
                .to_str()
                .is_some_and(|arg| arg.starts_with("--features=") || arg.starts_with("-F"))
    })
}

fn validate_selectors(args: &[OsString]) -> Result<(), String> {
    if args.iter().any(|arg| arg == "--all") {
        return Err(
            "graph does not accept Cargo's deprecated --all selector; use --workspace".into(),
        );
    }
    if args.iter().any(|arg| arg == "--workspace") && has_feature_selection(args) {
        return Err(FEATURE_SELECTION_ERROR.into());
    }
    let mut binaries = 0;
    let mut packages = 0;
    let mut targets = 0;
    for arg in args.iter().filter_map(|arg| arg.to_str()) {
        if arg == "--bin" || arg.starts_with("--bin=") {
            binaries += 1;
        }
        if arg == "-p" || arg == "--package" || arg.starts_with("--package=") {
            packages += 1;
        }
        if arg == "--target" || arg.starts_with("--target=") {
            targets += 1;
        }
        if arg.starts_with("-p") && arg != "-p" {
            return Err(
                "graph requires a separated -p PACKAGE or --package=PACKAGE selector".into(),
            );
        }
        if [
            "--tests",
            "--test",
            "--examples",
            "--example",
            "--all-targets",
            "--lib",
            "--bins",
            "--benches",
            "--bench",
            "--message-format",
            "--exclude",
        ]
        .iter()
        .any(|flag| arg == *flag || arg.starts_with(&format!("{flag}=")))
        {
            return Err(
                "graph selects binary entries and manages its JSON build output; use -p PACKAGE, --workspace, or --bin NAME".into(),
            );
        }
    }
    if packages != 0 && args.iter().any(|arg| arg == "--workspace") {
        return Err("graph accepts either --workspace or a package selector, not both".into());
    }
    if binaries > 1 {
        return Err("graph accepts only one --bin selector".into());
    }
    if packages > 1 {
        return Err("graph accepts only one package selector".into());
    }
    if targets > 1 {
        return Err("graph accepts only one --target selector".into());
    }
    Ok(())
}

fn extract_output(args: Vec<OsString>) -> Result<(Vec<OsString>, Option<PathBuf>), String> {
    let mut cargo = Vec::new();
    let mut output = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            cargo.push(arg);
            cargo.extend(args);
            break;
        }
        if arg == "--output" {
            output = Some(PathBuf::from(
                args.next()
                    .filter(|arg| !arg.is_empty() && arg != "--")
                    .ok_or("--output requires a file path")?,
            ));
        } else if let Some(path) = arg.to_str().and_then(|arg| arg.strip_prefix("--output=")) {
            if path.is_empty() {
                return Err("--output requires a file path".into());
            }
            output = Some(PathBuf::from(path));
        } else {
            cargo.push(arg);
        }
    }
    Ok((cargo, output))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn colored_cargo_diagnostics_retain_required_feature_errors_and_unicode() {
        assert_eq!(
            plain_diagnostic(
                "\u{1b}[1;31merror\u{1b}[0m: target `app` requires the features: `extra`\n位置：中文.rs"
            ),
            "error: target `app` requires the features: `extra`\n位置：中文.rs"
        );
        assert_eq!(
            plain_diagnostic("\u{1b}]8;;file:///source.rs\u{1b}\\source.rs\u{1b}]8;;\u{7}"),
            "source.rs"
        );
    }

    fn project_metadata() -> serde_json::Value {
        serde_json::json!({
            "workspace_members": ["app@1", "other@1", "library@1"],
            "workspace_default_members": ["app@1"],
            "packages": [
                {"id":"other@1", "name":"other", "version":"1", "manifest_path":"/other/Cargo.toml",
                 "targets":[{"name":"entry", "kind":["bin"], "src_path":"/other/main.rs"}]},
                {"id":"library@1", "name":"library", "version":"1", "manifest_path":"/lib/Cargo.toml", "targets":[]},
                {"id":"app@1", "name":"app", "version":"1", "manifest_path":"/app/Cargo.toml", "default_run":"entry",
                 "targets":[
                    {"name":"worker", "kind":["bin"], "src_path":"/app/worker.rs", "required-features":["worker"]},
                    {"name":"unit", "kind":["test"], "src_path":"/app/test.rs"},
                    {"name":"entry", "kind":["bin"], "src_path":"/app/main.rs"}
                 ]}
            ]
        })
    }

    #[test]
    fn project_selection_includes_non_default_binaries_and_retains_feature_gates() {
        let selection = select_targets(&project_metadata(), &arguments(&["-p", "app"])).unwrap();
        assert_eq!(
            selection
                .targets
                .iter()
                .map(|target| target.binary.as_str())
                .collect::<Vec<_>>(),
            ["entry", "worker"]
        );
        assert_eq!(selection.targets[1].required_features, ["worker"]);
        assert_eq!(selection.packages.len(), 1);
    }

    #[test]
    fn default_members_cannot_silently_change_multi_package_feature_selection() {
        let mut metadata = project_metadata();
        metadata["workspace_default_members"] = serde_json::json!(["app@1", "other@1"]);
        for flags in [
            vec!["--features", "worker"],
            vec!["--features=worker"],
            vec!["-F", "worker"],
            vec!["-Fworker"],
        ] {
            assert_eq!(
                select_targets(&metadata, &arguments(&flags))
                    .err()
                    .as_deref(),
                Some(FEATURE_SELECTION_ERROR),
            );
        }
        assert!(select_targets(&metadata, &arguments(&["--all-features"])).is_ok());
        assert!(
            select_targets(
                &metadata,
                &arguments(&["-p", "app", "--features", "worker"])
            )
            .is_ok()
        );
        assert!(select_targets(&project_metadata(), &arguments(&["--features", "worker"])).is_ok());
    }

    #[test]
    fn workspace_keeps_same_named_binaries_separate_and_does_not_invent_library_entries() {
        let selection = select_targets(&project_metadata(), &arguments(&["--workspace"])).unwrap();
        assert_eq!(selection.packages.len(), 3);
        assert_eq!(selection.targets.len(), 3);
        assert_eq!(selection.targets[0].binary, selection.targets[2].binary);
        assert_ne!(
            selection.targets[0].package_id,
            selection.targets[2].package_id
        );
        assert!(
            select_targets(
                &project_metadata(),
                &arguments(&["--workspace", "--bin", "entry"])
            )
            .is_err()
        );
        let entry = select_targets(
            &project_metadata(),
            &arguments(&["-p", "other", "--bin", "entry"]),
        )
        .unwrap();
        assert_eq!(entry.targets.len(), 1);
        assert_eq!(entry.targets[0].package_id, "other@1");
        assert!(select_targets(&project_metadata(), &arguments(&["-p", "library"])).is_err());
    }

    #[test]
    fn each_build_discards_broad_selectors_but_preserves_feature_profile_and_path_arguments() {
        let args = arguments(&[
            "--workspace",
            "--package=app",
            "--bin",
            "entry",
            "--target=host",
            "--manifest-path",
            "project with spaces/Cargo.toml",
            "--features",
            "feature-alias",
            "--no-default-features",
            "--release",
            "--config",
            "build.jobs=2",
        ]);
        assert_eq!(
            per_entry_arguments(&args).unwrap(),
            arguments(&[
                "--manifest-path",
                "project with spaces/Cargo.toml",
                "--features",
                "feature-alias",
                "--no-default-features",
                "--release",
                "--config",
                "build.jobs=2"
            ])
        );
    }

    #[test]
    fn mixed_target_selectors_cannot_populate_the_graph_cache_with_business_binaries() {
        for flags in [
            vec!["--all", "-p", "app", "--bin", "entry"],
            vec!["--bins", "--bin", "app"],
            vec!["--bin=a", "--bin", "b"],
            vec!["--test=x"],
            vec!["--example=y"],
            vec!["--bench=z"],
            vec!["--all-targets"],
            vec!["--target=a", "--target", "b"],
            vec!["--workspace", "--features", "app/worker"],
            vec!["--workspace", "-Fworker"],
        ] {
            let args = flags.into_iter().map(OsString::from).collect::<Vec<_>>();
            assert!(validate_selectors(&args).is_err());
        }
        assert!(validate_selectors(&["--bin=app".into()]).is_ok());
    }
}
