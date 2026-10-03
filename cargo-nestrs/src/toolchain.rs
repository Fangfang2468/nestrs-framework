//! Selection and validation of the exact compiler supported by this driver.

use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};

use crate::bridge::Bridge;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerIdentity {
    pub release: String,
    pub commit: String,
    pub host: String,
}

impl CompilerIdentity {
    pub fn pinned() -> Result<Self, String> {
        Self::for_build_host(include_str!("../toolchain.json"), env!("NESTRS_BUILD_HOST"))
    }

    fn for_build_host(value: &str, build_host: &str) -> Result<Self, String> {
        let json: serde_json::Value = serde_json::from_str(value)
            .map_err(|error| format!("invalid compiler identity JSON: {error}"))?;
        let hosts = json
            .get("hosts")
            .and_then(serde_json::Value::as_array)
            .ok_or("compiler identity is missing the supported hosts list")?;
        if !hosts.iter().any(|host| host.as_str() == Some(build_host)) {
            return Err(format!(
                "unsupported tool host {build_host}; supported hosts: {}",
                hosts
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .collect::<Vec<_>>()
                    .join(", "),
            ));
        }
        let field = |name: &str| {
            json.get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("compiler identity is missing {name}"))
        };
        Ok(Self {
            release: field("release")?,
            commit: field("commit_hash")?,
            host: build_host.to_owned(),
        })
    }

    /// Exact identity of this compiled tool, for validating a sibling driver.
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "release": self.release,
            "commit_hash": self.commit,
            "host": self.host,
        })
        .to_string()
    }

    fn from_json(value: &str) -> Result<Self, String> {
        let json: serde_json::Value = serde_json::from_str(value)
            .map_err(|error| format!("invalid compiler identity JSON: {error}"))?;
        let field = |name: &str| {
            json.get(name)
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("compiler identity is missing {name}"))
        };
        Ok(Self {
            release: field("release")?,
            commit: field("commit_hash")?,
            host: field("host")?,
        })
    }

    pub fn parse(output: &str) -> Result<Self, String> {
        let field = |name: &str| {
            output
                .lines()
                .find_map(|line| {
                    line.strip_prefix(name)
                        .and_then(|tail| tail.strip_prefix(": "))
                })
                .map(str::to_owned)
                .ok_or_else(|| format!("rustc -vV did not report {name}"))
        };
        Ok(Self {
            release: field("release")?,
            commit: field("commit-hash")?,
            host: field("host")?,
        })
    }

    pub fn verify(&self, actual: &Self) -> Result<(), String> {
        if actual != self {
            return Err(format!(
                "unsupported compiler: expected rustc {} ({}), host {}; found rustc {} ({}), host {}",
                self.release, self.commit, self.host, actual.release, actual.commit, actual.host,
            ));
        }
        Ok(())
    }

    pub fn cache_key(&self) -> String {
        format!("{}-{}-{}", self.release, self.commit, self.host)
    }
}

#[derive(Debug)]
pub struct Toolchain {
    pub identity: CompilerIdentity,
    pub rustc: PathBuf,
    pub sysroot: PathBuf,
    pub driver: PathBuf,
    pub bridge: PathBuf,
    pub fingerprint: String,
}

impl Toolchain {
    /// Namespace Cargo outputs by the exact compiler and both tool artifacts.
    /// Keep the Windows segment short enough for MSVC's linker path limit.
    pub fn cache_directory(&self, target: &Path) -> PathBuf {
        let base = target.join("nestrs");
        if self.identity.host.contains("windows") {
            let mut hash = 0xcbf29ce484222325_u64;
            for field in [
                self.identity.release.as_str(),
                self.identity.commit.as_str(),
                self.identity.host.as_str(),
                self.fingerprint.as_str(),
            ] {
                for byte in field.bytes().chain(std::iter::once(0)) {
                    hash ^= u64::from(byte);
                    hash = hash.wrapping_mul(0x100000001b3);
                }
            }
            base.join(format!("{hash:016x}"))
        } else {
            base.join(self.identity.cache_key()).join(&self.fingerprint)
        }
    }

    pub fn discover() -> Result<Self, String> {
        let identity = CompilerIdentity::pinned()?;
        let rustc = find_compiler(&identity)?;
        let sysroot_output = capture(&rustc, &["--print", "sysroot"])?;
        let sysroot = PathBuf::from(sysroot_output.trim());
        check_development_libraries(&sysroot, &identity.host)?;
        let driver = match env::var_os("NESTRS_DRIVER") {
            Some(path) => PathBuf::from(path),
            None => env::current_exe()
                .map_err(|error| format!("cannot locate cargo-nestrs: {error}"))?
                .with_file_name(format!("nestrs-driver{}", env::consts::EXE_SUFFIX)),
        };
        let driver = driver.canonicalize().map_err(|error| {
            format!(
                "cannot find compiler driver {}: {error}; install both cargo-nestrs and nestrs-driver, or set NESTRS_DRIVER",
                driver.display(),
            )
        })?;
        if !driver.is_file() {
            return Err(format!(
                "compiler driver is not a file: {}",
                driver.display()
            ));
        }
        // Cargo does not track wrapper or injected proc-macro contents. Either
        // artifact changing must invalidate the same isolated build namespace.
        let bridge = Bridge::discover(&driver)?;
        let fingerprint = format!("{}-{}", fingerprint(&driver)?, bridge.fingerprint);
        let toolchain = Self {
            identity,
            rustc,
            sysroot,
            driver,
            bridge: bridge.path,
            fingerprint,
        };
        toolchain.verify_driver()?;
        Ok(toolchain)
    }

    pub fn configure(&self, command: &mut Command) -> Result<(), String> {
        command
            .env("RUSTC", &self.rustc)
            // The wrapper adds the same private macro bridge for actual rustdoc
            // and doctest compilation, then delegates to the pinned rustdoc.
            .env("RUSTDOC", &self.driver)
            .env(
                "NESTRS_REAL_RUSTDOC",
                self.sysroot
                    .join("bin")
                    .join(format!("rustdoc{}", env::consts::EXE_SUFFIX)),
            )
            .env("NESTRS_MACRO_BRIDGE", &self.bridge)
            .env("RUSTC_WRAPPER", &self.driver)
            // An empty override also disables wrappers from .cargo/config.toml.
            .env("RUSTC_WORKSPACE_WRAPPER", "")
            .env_remove("RUSTC_BOOTSTRAP")
            // 编辑器模型只能选择 RA 的展示分支；真实编译必须重新执行语义关联，
            // 不能继承宿主环境中的旧模型而提前裁剪 constructor 候选。
            .env_remove(crate::ide::constructor::MODEL_ENV)
            .env_remove("NESTRS_GRAPH_TARGET")
            .env_remove("NESTRS_GRAPH_BINARY")
            .env_remove("NESTRS_GRAPH_MANIFEST")
            .env_remove("NESTRS_GRAPH_PROOF")
            .env_remove("NESTRS_GRAPH_SOURCE")
            .env_remove("NESTRS_GRAPH_PLAN")
            .env(library_path_variable(), self.library_path()?)
            .env("CARGO_INCREMENTAL", "0")
            .env("NESTRS_TOOLCHAIN_ID", self.identity.cache_key());
        Ok(())
    }

    fn library_path(&self) -> Result<OsString, String> {
        let mut paths = runtime_library_directories(&self.sysroot, &self.identity.host);
        if let Some(current) = env::var_os(library_path_variable()) {
            paths.extend(env::split_paths(&current));
        }
        env::join_paths(paths)
            .map_err(|error| format!("cannot prepare driver library search path: {error}"))
    }

    fn verify_driver(&self) -> Result<(), String> {
        let output = Command::new(&self.driver)
            .arg("--nestrs-driver-info")
            .env(library_path_variable(), self.library_path()?)
            .output()
            .map_err(|error| {
                format!(
                    "cannot start compiler driver {}: {error}",
                    self.driver.display()
                )
            })?;
        if !output.status.success() {
            return Err(format!(
                "compiler driver identity check failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let json = std::str::from_utf8(&output.stdout)
            .map_err(|error| format!("invalid compiler driver identity: {error}"))?;
        let identity = CompilerIdentity::from_json(json)?;
        self.identity
            .verify(&identity)
            .map_err(|error| format!("installed compiler driver is incompatible: {error}"))
    }
}

fn find_compiler(identity: &CompilerIdentity) -> Result<PathBuf, String> {
    if let Some(path) = env::var_os("NESTRS_RUSTC") {
        return validate_compiler(&direct_program_path(Path::new(&path))?, identity);
    }
    let mut failures = Vec::new();
    // `rustup which --toolchain <missing>` may install that toolchain. Listing
    // existing paths is read-only and avoids running any rustup compiler proxy.
    if let Ok(output) = Command::new("rustup")
        .args(["toolchain", "list", "--verbose"])
        .output()
        && output.status.success()
        && let Ok(installed) = std::str::from_utf8(&output.stdout)
    {
        let compilers = installed_compilers(installed, identity);
        for path in compilers {
            match validate_compiler(&path, identity) {
                Ok(rustc) => return Ok(rustc),
                Err(error) => failures.push(error),
            }
        }
    }
    match direct_program_path(Path::new("rustc"))
        .and_then(|path| validate_compiler(&path, identity))
    {
        Ok(rustc) => Ok(rustc),
        Err(error) => {
            failures.push(error);
            Err(format!(
                "cannot locate the supported compiler; install rustc {} with rustc-dev, or set NESTRS_RUSTC to that compiler. {}",
                identity.release,
                failures.join("; "),
            ))
        }
    }
}

fn installed_compilers(output: &str, identity: &CompilerIdentity) -> Vec<PathBuf> {
    let mut installed = Vec::new();
    for line in output.lines() {
        let Some((name, rest)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start();
        let directory = if rest.starts_with('(') {
            let Some((_, path)) = rest.split_once(')') else {
                continue;
            };
            path.trim_start()
        } else {
            rest
        };
        let rank = if name == identity.release
            || name == format!("{}-{}", identity.release, identity.host)
        {
            0
        } else if name == "stable" || name == format!("stable-{}", identity.host) {
            1
        } else {
            continue;
        };
        let compiler = Path::new(directory)
            .join("bin")
            .join(format!("rustc{}", env::consts::EXE_SUFFIX));
        if compiler.is_file() {
            installed.push((rank, compiler));
        }
    }
    installed.sort();
    installed.into_iter().map(|(_, path)| path).collect()
}

fn executable_candidates(program: &Path, suffix: &str) -> Vec<PathBuf> {
    let mut paths = vec![program.to_owned()];
    if !suffix.is_empty() && program.extension().is_none() {
        let mut name = program.as_os_str().to_owned();
        name.push(suffix);
        paths.push(PathBuf::from(name));
    }
    paths
}

fn direct_program_path(program: &Path) -> Result<PathBuf, String> {
    let candidates = executable_candidates(program, env::consts::EXE_SUFFIX);
    let resolved = if program.components().count() > 1 || program.is_absolute() {
        candidates.into_iter().find(|path| path.is_file())
    } else {
        env::var_os("PATH").and_then(|paths| {
            env::split_paths(&paths)
                .flat_map(|directory| {
                    candidates
                        .iter()
                        .map(move |program| directory.join(program))
                })
                .find(|path| path.is_file())
        })
    }
    .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "not found in PATH"))
    .and_then(|path| path.canonicalize())
    .map_err(|error| format!("cannot resolve compiler {}: {error}", program.display()))?;
    if is_rustup_proxy(&resolved)? {
        return Err("a rustup compiler proxy cannot be used without an installed matching toolchain; set NESTRS_RUSTC to an actual sysroot/bin/rustc".into());
    }
    Ok(resolved)
}

fn is_rustup_proxy(program: &Path) -> Result<bool, String> {
    if program.file_stem().is_some_and(|name| name == "rustup") {
        return Ok(true);
    }
    // rustup may use hard links or copies for rustc.exe on Windows. Canonicalize
    // only detects symlinks, so compare with its adjacent rustup executable
    // before invoking a proxy that could install a missing toolchain.
    let rustup = program.with_file_name(format!("rustup{}", env::consts::EXE_SUFFIX));
    let Ok(rustup_metadata) = rustup.metadata() else {
        return Ok(false);
    };
    let metadata = program.metadata().map_err(|error| error.to_string())?;
    Ok(metadata.len() == rustup_metadata.len() && fingerprint(program)? == fingerprint(&rustup)?)
}

pub(crate) fn library_path_variable() -> &'static str {
    if cfg!(windows) {
        "PATH"
    } else {
        "LD_LIBRARY_PATH"
    }
}

pub(crate) fn runtime_library_directories(sysroot: &Path, host: &str) -> Vec<PathBuf> {
    if host.contains("windows") {
        vec![
            sysroot.join("bin"),
            sysroot.join("lib").join("rustlib").join(host).join("lib"),
        ]
    } else {
        vec![sysroot.join("lib")]
    }
}

fn validate_compiler(path: &Path, expected: &CompilerIdentity) -> Result<PathBuf, String> {
    let actual = CompilerIdentity::parse(&capture(path, &["-vV"])?)?;
    expected.verify(&actual)?;
    // Resolve the real compiler through sysroot so a rustup proxy cannot switch
    // toolchains between the identity check and the subsequent Cargo command.
    let sysroot = capture(path, &["--print", "sysroot"])?;
    let compiler = Path::new(sysroot.trim())
        .join("bin")
        .join(format!("rustc{}", env::consts::EXE_SUFFIX));
    let compiler = compiler.canonicalize().map_err(|error| {
        format!(
            "cannot locate validated rustc {}: {error}",
            compiler.display()
        )
    })?;
    let resolved = CompilerIdentity::parse(&capture(&compiler, &["-vV"])?)?;
    expected.verify(&resolved)?;
    Ok(compiler)
}

fn check_development_libraries(sysroot: &Path, host: &str) -> Result<(), String> {
    let metadata = sysroot.join("lib").join("rustlib").join(host).join("lib");
    let entries = fs::read_dir(&metadata)
        .map_err(|error| {
            format!(
                "cannot read compiler libraries {}: {error}",
                metadata.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot read compiler libraries: {error}"))?;
    for component in ["librustc_middle-", "librustc_interface-"] {
        if !entries.iter().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(component) && name.ends_with(".rmeta")
        }) {
            return Err(format!(
                "matching rustc-dev is missing from {}; install rustc-dev for the pinned toolchain",
                sysroot.display(),
            ));
        }
    }
    Ok(())
}

pub(crate) fn capture(program: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    // A direct rustc.exe needs its adjacent compiler DLLs even during the first
    // identity/sysroot query, before Toolchain exists and can prepare the full
    // library search path.
    if cfg!(windows)
        && let Some(directory) = program.parent()
        && !directory.as_os_str().is_empty()
    {
        let mut paths = vec![directory.to_owned()];
        if let Some(current) = env::var_os("PATH") {
            paths.extend(env::split_paths(&current));
        }
        command.env(
            "PATH",
            env::join_paths(paths)
                .map_err(|error| format!("cannot prepare compiler DLL search path: {error}"))?,
        );
    }
    let output = command
        .output()
        .map_err(|error| format!("cannot run {}: {error}", program.display()))?;
    if !output.status.success() {
        return Err(format!(
            "{} {} failed: {}",
            program.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{} returned invalid UTF-8: {error}", program.display()))
}

fn fingerprint(path: &Path) -> Result<String, String> {
    let mut file = File::open(path)
        .map_err(|error| format!("cannot read compiler driver {}: {error}", path.display()))?;
    let mut hash = 0xcbf29ce484222325_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let length = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot fingerprint compiler driver: {error}"))?;
        if length == 0 {
            break;
        }
        for byte in &buffer[..length] {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
    }
    Ok(format!("{hash:016x}"))
}

pub(crate) fn cargo_program() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "nestrs-toolchain-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn full_identity_includes_commit_and_host() {
        let pinned = CompilerIdentity::pinned().unwrap();
        let text = format!(
            "rustc {}\nbinary: rustc\ncommit-hash: {}\nhost: {}\nrelease: {}\n",
            pinned.release, pinned.commit, pinned.host, pinned.release
        );
        let parsed = CompilerIdentity::parse(&text).unwrap();
        pinned.verify(&parsed).unwrap();
        let mut changed = parsed.clone();
        changed.commit.push('0');
        assert!(pinned.verify(&changed).is_err());
        let mut changed = parsed;
        changed.host.push_str("-other");
        assert!(pinned.verify(&changed).is_err());
    }

    #[test]
    fn abbreviated_version_cannot_pass_identity_validation() {
        assert!(CompilerIdentity::parse("rustc 1.98.0\nrelease: 1.98.0\n").is_err());
    }

    #[test]
    fn supported_hosts_do_not_allow_a_driver_to_change_its_build_host() {
        let pin = include_str!("../toolchain.json");
        let linux = CompilerIdentity::for_build_host(pin, "x86_64-unknown-linux-gnu").unwrap();
        let windows = CompilerIdentity::for_build_host(pin, "x86_64-pc-windows-msvc").unwrap();
        assert!(linux.verify(&windows).is_err());
        assert!(windows.verify(&linux).is_err());
        assert!(CompilerIdentity::for_build_host(pin, "x86_64-pc-windows-gnu").is_err());
        assert_ne!(linux.cache_key(), windows.cache_key());
        assert_eq!(
            CompilerIdentity::from_json(&windows.to_json()).unwrap(),
            windows
        );
        assert_eq!(
            CompilerIdentity::pinned().unwrap().host,
            env!("NESTRS_BUILD_HOST")
        );
    }

    #[test]
    fn installed_compilers_preserve_space_paths_and_rank_matching_release_first() {
        let fixture = Fixture::new();
        let identity = CompilerIdentity::pinned().unwrap();
        let stable = fixture.0.join("stable path");
        let pinned = fixture.0.join("pinned path");
        for directory in [&stable, &pinned] {
            fs::create_dir_all(directory.join("bin")).unwrap();
            fs::write(
                directory
                    .join("bin")
                    .join(format!("rustc{}", env::consts::EXE_SUFFIX)),
                "compiler",
            )
            .unwrap();
        }
        let listing = format!(
            "stable-{} (active, default) {}\n{}-{} {}\n",
            identity.host,
            stable.display(),
            identity.release,
            identity.host,
            pinned.display(),
        );
        let result = installed_compilers(&listing, &identity);
        assert_eq!(result.len(), 2);
        assert!(result[0].starts_with(&pinned));
        assert!(result[1].starts_with(&stable));
    }

    #[test]
    fn direct_compiler_lookup_rejects_copied_rustup_proxies() {
        let fixture = Fixture::new();
        let compiler = fixture.0.join(format!("rustc{}", env::consts::EXE_SUFFIX));
        let rustup = fixture.0.join(format!("rustup{}", env::consts::EXE_SUFFIX));
        fs::write(&compiler, "rustup proxy").unwrap();
        fs::copy(&compiler, &rustup).unwrap();
        assert!(
            direct_program_path(&compiler)
                .unwrap_err()
                .contains("rustup compiler proxy")
        );
        fs::write(&compiler, "real rustc").unwrap();
        assert_eq!(
            direct_program_path(&compiler).unwrap(),
            compiler.canonicalize().unwrap()
        );
    }

    #[test]
    fn windows_program_candidates_accept_bare_and_explicit_exe_paths() {
        assert_eq!(
            executable_candidates(Path::new("rustc"), ".exe"),
            vec![PathBuf::from("rustc"), PathBuf::from("rustc.exe")],
        );
        assert_eq!(
            executable_candidates(Path::new("rustc.exe"), ".exe"),
            vec![PathBuf::from("rustc.exe")],
        );
        assert_eq!(
            executable_candidates(Path::new("rustc"), ""),
            vec![PathBuf::from("rustc")],
        );
    }

    #[test]
    fn compact_windows_cache_retains_compiler_and_both_tool_identities() {
        let mut toolchain = Toolchain {
            identity: CompilerIdentity::for_build_host(
                include_str!("../toolchain.json"),
                "x86_64-pc-windows-msvc",
            )
            .unwrap(),
            rustc: PathBuf::new(),
            sysroot: PathBuf::new(),
            driver: PathBuf::new(),
            bridge: PathBuf::new(),
            fingerprint: "1111111111111111-2222222222222222".into(),
        };
        let target = Path::new("target");
        let original = toolchain.cache_directory(target);
        assert_eq!(original.file_name().unwrap(), "01c90ba81b78b681");
        assert_eq!(original.parent(), Some(target.join("nestrs").as_path()));
        assert_eq!(original.file_name().unwrap().to_str().unwrap().len(), 16);
        assert_eq!(toolchain.cache_directory(target), original);

        toolchain.identity.commit.push('0');
        assert_ne!(toolchain.cache_directory(target), original);
        toolchain.identity.commit.pop();
        toolchain.identity.release.push('0');
        assert_ne!(toolchain.cache_directory(target), original);
        toolchain.identity.release.pop();
        toolchain.fingerprint = "3333333333333333-2222222222222222".into();
        assert_ne!(toolchain.cache_directory(target), original);
        toolchain.fingerprint = "1111111111111111-4444444444444444".into();
        assert_ne!(toolchain.cache_directory(target), original);
        toolchain.fingerprint = "1111111111111111-2222222222222222".into();
        toolchain.identity.host = "x86_64-unknown-linux-gnu".into();
        let linux = toolchain.cache_directory(target);
        assert_ne!(linux, original);
        assert_eq!(
            linux,
            target
                .join("nestrs")
                .join(toolchain.identity.cache_key())
                .join(&toolchain.fingerprint),
        );
    }

    #[test]
    fn real_compilation_drops_the_editor_only_constructor_model() {
        let toolchain = Toolchain {
            identity: CompilerIdentity::pinned().unwrap(),
            rustc: "rustc".into(),
            sysroot: "sysroot".into(),
            driver: "nestrs-driver".into(),
            bridge: "bridge".into(),
            fingerprint: String::new(),
        };
        let mut command = Command::new("cargo");
        command.env(crate::ide::constructor::MODEL_ENV, "stale-editor-model");
        toolchain.configure(&mut command).unwrap();
        assert_eq!(
            command
                .get_envs()
                .find(|(name, _)| *name == crate::ide::constructor::MODEL_ENV),
            Some((
                std::ffi::OsStr::new(crate::ide::constructor::MODEL_ENV),
                None
            )),
        );
    }
}
