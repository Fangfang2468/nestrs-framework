use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Value, json};

use super::capture::Unit;
use crate::toolchain::{Toolchain, capture};

struct Selected {
    unit: Unit,
    artifact: Value,
}

/// Build the editor graph from successful Cargo artifacts and exact rustc units.
/// Metadata is used only for workspace membership; dependency edges come from
/// the compiler's --extern paths, including renamed and host proc-macro crates.
pub(crate) fn generate(
    captures: &Path,
    messages: &[Value],
    metadata: &Value,
    toolchain: &Toolchain,
    target_directory: &Path,
    destination: &Path,
) -> Result<Value, String> {
    let mut units = Vec::new();
    for entry in fs::read_dir(captures)
        .map_err(|error| format!("missing IDE compilation records: {error}"))?
    {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().is_some_and(|ext| ext == "json") {
            units.push(
                serde_json::from_slice::<Unit>(&fs::read(path).map_err(|error| error.to_string())?)
                    .map_err(|error| format!("invalid IDE compilation record: {error}"))?,
            );
        }
    }
    units.sort_by(|a, b| {
        (&a.root_module, &a.out_dir, &a.extra_filename, a.test).cmp(&(
            &b.root_module,
            &b.out_dir,
            &b.extra_filename,
            b.test,
        ))
    });
    let mut selected = Vec::new();
    let mut seen = BTreeSet::new();
    for artifact in messages
        .iter()
        .filter(|value| value["reason"] == "compiler-artifact")
    {
        let root = PathBuf::from(required_str(&artifact["target"], "src_path")?);
        let root = root.canonicalize().unwrap_or(root);
        let test = artifact["profile"]["test"].as_bool().unwrap_or(false);
        let files = artifact_files(artifact)?;
        let candidates: Vec<_> = units
            .iter()
            .filter(|unit| unit.root_module == root && unit.test == test)
            .collect();
        let exact: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|unit| files.iter().any(|path| unit.owns_artifact(path)))
            .collect();
        let unit = match exact.as_slice() {
            [unit] => *unit,
            [] if candidates.len() == 1 => candidates[0],
            _ => {
                return Err(format!(
                    "cannot identify the exact compiler unit for {}; regenerate with a new --target-dir instead of using incomplete IDE data",
                    root.display(),
                ));
            }
        };
        let identity = (
            unit.out_dir.clone(),
            unit.crate_name.clone(),
            unit.extra_filename.clone(),
            unit.test,
        );
        if seen.insert(identity) {
            selected.push(Selected {
                unit: unit.clone(),
                artifact: artifact.clone(),
            });
        }
    }
    if selected.is_empty() {
        return Err("Cargo produced no Rust targets for the IDE project".into());
    }
    // The stock JSON project loader unions these host defaults with each
    // crate's cfg. Additive target features and profile debug_assertions are
    // representable, but removing a default would leave contradictory branches
    // active in the editor. Refuse that model instead of hiding the mismatch.
    let defaults = capture(
        &toolchain.rustc,
        &["--print", "cfg", "-O", "--target", &toolchain.identity.host],
    )
    .map_err(|error| format!("cannot query IDE host cfg: {error}"))?;
    for entry in &selected {
        let unit = &entry.unit;
        if unit
            .target
            .as_ref()
            .is_some_and(|target| target != &toolchain.identity.host)
        {
            return Err("cargo nestrs init currently supports the pinned host target only".into());
        }
        let removed: Vec<_> = defaults
            .lines()
            .filter(|cfg| !unit.cfg.iter().any(|actual| actual == cfg))
            .collect();
        if !removed.is_empty() {
            return Err(format!(
                "cannot represent compiler cfg for {} in stock rust-analyzer: its default cfg would re-enable {}; IDE preparation currently requires the default host panic strategy and target features",
                unit.crate_name,
                removed.join(", "),
            ));
        }
    }
    selected.sort_by(|a, b| {
        (&a.unit.root_module, a.unit.test, &a.unit.extra_filename).cmp(&(
            &b.unit.root_module,
            b.unit.test,
            &b.unit.extra_filename,
        ))
    });

    let mut artifacts = BTreeMap::new();
    for (index, entry) in selected.iter().enumerate() {
        for file in artifact_files(&entry.artifact)? {
            artifacts.insert(file, index);
        }
    }
    let workspace_members: BTreeSet<_> = metadata["workspace_members"]
        .as_array()
        .ok_or("Cargo metadata has no workspace members")?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    // Source sets must be identical for lib, binary, and test units sharing files.
    let mut sources: BTreeMap<PathBuf, BTreeSet<PathBuf>> = BTreeMap::new();
    for entry in &selected {
        let include = sources.entry(entry.unit.manifest_dir.clone()).or_default();
        include.insert(entry.unit.manifest_dir.clone());
        if let Some(out) = entry.unit.env.get("OUT_DIR") {
            include.insert(PathBuf::from(out));
        }
    }
    let bridge_index = selected.len();
    let mut crates = Vec::new();
    for entry in &selected {
        let unit = &entry.unit;
        let mut deps = Vec::new();
        for (name, path) in &unit.externs {
            let index = artifacts
                .get(path)
                .copied()
                .or_else(|| {
                    selected
                        .iter()
                        .position(|candidate| candidate.unit.owns_artifact(path))
                })
                .ok_or_else(|| {
                    format!(
                        "IDE dependency {name} of {} has no matching compiler unit: {}",
                        unit.crate_name,
                        path.display(),
                    )
                })?;
            deps.push(json!({"crate": index, "name": name}));
        }
        if unit.externs.contains_key("nestrs_core") {
            if unit.externs.contains_key("nestrs") {
                return Err("the `nestrs` dependency name is reserved for the tool-managed declaration bridge".into());
            }
            deps.push(json!({"crate": bridge_index, "name": "nestrs"}));
        }
        let mut environment = BTreeMap::<String, String>::new();
        for build in messages
            .iter()
            .filter(|value| value["reason"] == "build-script-executed")
        {
            let matches = unit
                .env
                .get("OUT_DIR")
                .is_some_and(|out| build["out_dir"].as_str() == Some(out))
                || (!unit.env.contains_key("OUT_DIR")
                    && build["package_id"] == entry.artifact["package_id"]);
            if matches && let Some(variables) = build["env"].as_array() {
                for pair in variables {
                    if let (Some(name), Some(value)) = (pair[0].as_str(), pair[1].as_str()) {
                        environment.insert(name.to_owned(), value.to_owned());
                    }
                }
            }
        }
        environment.extend(unit.env.clone());
        normalize_editor_environment(&mut environment)?;
        let package = required_str(&entry.artifact, "package_id")?;
        let include_dirs = sources[&unit.manifest_dir]
            .iter()
            .map(|path| editor_path(path))
            .collect::<Result<Vec<_>, _>>()?;
        let exclude_dirs = [
            editor_path(target_directory)?,
            editor_path(&unit.manifest_dir.join("target"))?,
            editor_path(&unit.manifest_dir.join(".git"))?,
        ];
        let mut node = json!({
            "display_name": unit.crate_name,
            "root_module": editor_path(&unit.root_module)?,
            "edition": unit.edition,
            "version": unit.env.get("CARGO_PKG_VERSION"),
            "deps": deps,
            "cfg": unit.cfg,
            "target": unit.target.as_ref().unwrap_or(&toolchain.identity.host),
            "env": environment,
            "is_workspace_member": workspace_members.contains(package),
            "is_proc_macro": unit.is_proc_macro(),
            "proc_macro_cwd": editor_path(&unit.manifest_dir)?,
            "source": {
                "include_dirs": include_dirs,
                "exclude_dirs": exclude_dirs
            }
        });
        if unit.is_proc_macro() {
            // A workspace proc-macro checked only as a top-level target may
            // have metadata but no dylib. It has no expansion consumers; Cargo
            // builds a separate host dylib unit when another target uses it.
            let library = artifact_files(&entry.artifact)?.into_iter().find(|path| {
                path.extension()
                    .is_some_and(|ext| ext == "so" || ext == "dylib" || ext == "dll")
            });
            if let Some(library) = library {
                node["proc_macro_dylib_path"] = json!(editor_path(&library)?);
            }
        }
        crates.push(node);
    }
    let bridge_source = destination
        .parent()
        .ok_or("IDE project output needs a parent directory")?
        .join("bridge")
        .join("lib.rs");
    super::write_atomic(
        &bridge_source,
        include_bytes!("../../internal/bridge/src/lib.rs"),
    )?;
    crates.push(json!({
        "display_name": "nestrs",
        "root_module": editor_path(&bridge_source)?,
        "edition": "2024",
        "deps": [], "cfg": [], "env": {},
        "is_workspace_member": false,
        "is_proc_macro": true,
        "proc_macro_dylib_path": editor_path(&toolchain.bridge)?,
    }));
    Ok(json!({
        "sysroot": editor_path(&toolchain.sysroot)?,
        "sysroot_src": editor_path(&toolchain.sysroot.join("lib/rustlib/src/rust/library"))?,
        "crates": crates,
    }))
}

fn normalize_editor_environment(environment: &mut BTreeMap<String, String>) -> Result<(), String> {
    // These Cargo values feed include!/env! path resolution. Business variables
    // retain their exact values, even when a value happens to look like a path.
    for name in ["OUT_DIR", "CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH"] {
        if let Some(value) = environment.get_mut(name) {
            *value = editor_path(Path::new(value))?
                .into_os_string()
                .into_string()
                .map_err(|_| format!("IDE path environment variable {name} is not UTF-8"))?;
        }
    }
    Ok(())
}

#[cfg(not(windows))]
fn editor_path(path: &Path) -> Result<PathBuf, String> {
    Ok(path.to_owned())
}

#[cfg(windows)]
fn editor_path(path: &Path) -> Result<PathBuf, String> {
    use std::{ffi::OsString, path::Component, path::Prefix};

    // Keep canonical filesystem paths for compiler/artifact matching above.
    // Stock rust-analyzer distinguishes Disk from VerbatimDisk in its VFS,
    // while editor didOpen URIs use ordinary drive/UNC paths.
    let unsupported = || {
        format!(
            "IDE cannot represent Windows device or verbatim-only path {}; use ordinary drive or UNC paths for editor projects",
            path.display()
        )
    };
    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return Ok(path.to_owned());
    };
    let mut ordinary = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:\\", char::from(drive))),
        Prefix::VerbatimUNC(server, share) => {
            if !ordinary_windows_component(server) || !ordinary_windows_component(share) {
                return Err(unsupported());
            }
            let mut root = OsString::from("\\\\");
            root.push(server);
            root.push("\\");
            root.push(share);
            PathBuf::from(root)
        }
        Prefix::Verbatim(_) | Prefix::DeviceNS(_) => return Err(unsupported()),
        _ => return Ok(path.to_owned()),
    };
    for component in components {
        match component {
            Component::RootDir => {}
            Component::Normal(name) if ordinary_windows_component(name) => ordinary.push(name),
            _ => return Err(unsupported()),
        }
    }
    Ok(ordinary)
}

#[cfg(windows)]
fn ordinary_windows_component(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    if name.is_empty()
        || name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|ch| ch.is_control() || "<>:\"/\\|?*".contains(ch))
    {
        return false;
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) && !["COM", "LPT"].iter().any(|prefix| {
        stem.strip_prefix(*prefix).is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        })
    })
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("Cargo message is missing {key}"))
}

fn artifact_files(value: &Value) -> Result<Vec<PathBuf>, String> {
    value["filenames"]
        .as_array()
        .ok_or("Cargo artifact has no filenames")?
        .iter()
        .map(|value| {
            value
                .as_str()
                // Cargo's JSON paths use ordinary drive paths on Windows;
                // rustc captures canonicalize to verbatim paths. Compare the
                // same filesystem identities for both sides of each edge.
                .map(|path| {
                    let path = PathBuf::from(path);
                    path.canonicalize().unwrap_or(path)
                })
                .ok_or_else(|| "invalid Cargo artifact filename".into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn editor_paths_unify_verbatim_drive_and_unc_sources_with_editor_uris() {
        for (source, expected) in [
            (
                r"\\?\C:\project with spaces\src\main.rs",
                r"C:\project with spaces\src\main.rs",
            ),
            (
                r"\\?\UNC\server\share\项目\src\main.rs",
                r"\\server\share\项目\src\main.rs",
            ),
            (r"C:\project\src\main.rs", r"C:\project\src\main.rs"),
        ] {
            assert_eq!(editor_path(Path::new(source)).unwrap(), Path::new(expected));
        }
    }

    #[cfg(windows)]
    #[test]
    fn editor_paths_reject_device_and_verbatim_only_filename_semantics() {
        for source in [
            r"\\.\C:\project\main.rs",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\main.rs",
            r"\\?\C:\project.\main.rs",
            r"\\?\C:\project \main.rs",
            r"\\?\C:\project\NUL.rs",
            r"\\?\C:\project\COM1.txt",
            r"\\?\C:\project\..\main.rs",
            r"\\?\C:\project\main.rs:stream",
        ] {
            assert!(editor_path(Path::new(source)).is_err(), "{source}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn only_cargo_path_environment_values_are_normalized_for_the_editor() {
        let original = r"\\?\C:\project with spaces\target\out";
        let mut environment = BTreeMap::from([
            ("OUT_DIR".into(), original.into()),
            ("CARGO_MANIFEST_DIR".into(), original.into()),
            ("CARGO_MANIFEST_PATH".into(), original.into()),
            ("BUSINESS_SETTING".into(), original.into()),
        ]);
        normalize_editor_environment(&mut environment).unwrap();
        for name in ["OUT_DIR", "CARGO_MANIFEST_DIR", "CARGO_MANIFEST_PATH"] {
            assert_eq!(environment[name], r"C:\project with spaces\target\out");
        }
        assert_eq!(environment["BUSINESS_SETTING"], original);
    }

    #[cfg(not(windows))]
    #[test]
    fn editor_paths_leave_unix_names_and_environment_values_unchanged() {
        let path = Path::new(r"/project/\\?\C:\literal/main.rs");
        assert_eq!(editor_path(path).unwrap(), path);
        let mut environment = BTreeMap::from([("OUT_DIR".into(), "/project/target/out".into())]);
        normalize_editor_environment(&mut environment).unwrap();
        assert_eq!(environment["OUT_DIR"], "/project/target/out");
    }
}
