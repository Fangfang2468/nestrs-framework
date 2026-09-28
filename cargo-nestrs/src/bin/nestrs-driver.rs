//! Version-pinned Nestrs compiler driver for semantic automatic binding.
//!
//! Cargo invokes this as a RUSTC_WRAPPER. Semantic analysis discovers
//! pairs; a fresh compiler invocation checks generated ordinary Rust through a
//! virtual source overlay. No application source file is rewritten.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_parse;
extern crate rustc_session;
extern crate rustc_span;

#[path = "../compiler/autobind_codegen.rs"]
mod autobind_codegen;
#[path = "../compiler/autobind_semantic.rs"]
mod autobind_semantic;

use autobind_codegen::OverlayFileLoader;
use autobind_semantic::Analysis;
use cargo_nestrs::bridge::{Bridge, has_extern, inject_dependency_search, inject_extern};
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_span::source_map::{FileLoader, RealFileLoader};
use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    fmt::Write as _,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::{Arc, Mutex},
};

type Sources = Arc<Mutex<BTreeMap<PathBuf, String>>>;

struct SnapshotLoader(Sources);

impl FileLoader for SnapshotLoader {
    fn file_exists(&self, path: &Path) -> bool {
        RealFileLoader.file_exists(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        let contents = RealFileLoader.read_file(path)?;
        let mut sources = self.0.lock().expect("source snapshot lock poisoned");
        let canonical = path.canonicalize()?;
        if let Some(original) = sources.get(&canonical) {
            if original != &contents {
                return Err(source_changed(path));
            }
        } else {
            sources.insert(canonical, contents.clone());
        }
        Ok(contents)
    }

    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        RealFileLoader.read_binary_file(path)
    }

    fn current_directory(&self) -> io::Result<PathBuf> {
        RealFileLoader.current_directory()
    }
}

struct Discover {
    analysis: Option<Result<Analysis, String>>,
    sources: Sources,
}

impl Callbacks for Discover {
    fn config(&mut self, config: &mut interface::Config) {
        // Generated bindings can make previously unused declarations live.
        // The final compiler invocation remains the authority for all lints.
        config.opts.lint_cap = Some(rustc_session::lint::Level::Allow);
        // Downstream `cargo check` targets also need closed blueprint MIR;
        // rustc otherwise omits it from metadata-only compilations.
        config.opts.unstable_opts.always_encode_mir = true;
        config.file_loader = Some(Box::new(SnapshotLoader(self.sources.clone())));
    }

    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        self.analysis = Some(autobind_semantic::analyze(tcx));
        Compilation::Stop
    }
}

struct Generate {
    loader: Option<OverlayFileLoader>,
    snapshots: BTreeMap<PathBuf, String>,
    expected: (usize, usize, usize),
    validation: Option<Result<(), String>>,
}

impl Callbacks for Generate {
    fn config(&mut self, config: &mut interface::Config) {
        // Downstream `cargo check` targets also need closed blueprint MIR;
        // rustc otherwise omits it from metadata-only compilations.
        config.opts.unstable_opts.always_encode_mir = true;
        config.file_loader = self.loader.take().map(|loader| {
            Box::new(CheckedLoader {
                loader,
                snapshots: self.snapshots.clone(),
            }) as Box<dyn FileLoader + Send + Sync>
        });
    }

    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        self.validation = Some(autobind_semantic::analyze(tcx).and_then(|analysis| {
            let observed = (analysis.providers, analysis.requests, analysis.explicit_bindings);
            if analysis.generated_bindings != 0 || observed != self.expected {
                Err(format!(
                    "DI semantic inputs changed between compiler passes: expected {:?}, observed {:?}, still missing {} bindings",
                    self.expected, observed, analysis.generated_bindings,
                ))
            } else {
                Ok(())
            }
        }));
        if self.validation.as_ref().is_some_and(Result::is_ok) {
            Compilation::Continue
        } else {
            Compilation::Stop
        }
    }
}

struct CheckedLoader {
    loader: OverlayFileLoader,
    snapshots: BTreeMap<PathBuf, String>,
}

impl FileLoader for CheckedLoader {
    fn file_exists(&self, path: &Path) -> bool {
        self.loader.file_exists(path)
    }

    fn read_file(&self, path: &Path) -> io::Result<String> {
        let canonical = path.canonicalize()?;
        let original = self.snapshots.get(&canonical).ok_or_else(|| io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Nestrs second compilation introduced a source file absent from semantic analysis: {}", path.display()),
        ))?;
        if &RealFileLoader.read_file(path)? != original {
            return Err(source_changed(path));
        }
        self.loader.read_file(path)
    }

    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        self.loader.read_binary_file(path)
    }

    fn current_directory(&self) -> io::Result<PathBuf> {
        self.loader.current_directory()
    }
}

fn source_changed(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Nestrs source changed between compiler reads: {}",
            path.display()
        ),
    )
}

fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--nestrs-driver-info") {
        return match cargo_nestrs::toolchain::CompilerIdentity::pinned() {
            Ok(identity) => {
                println!("{}", identity.to_json());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: Nestrs automatic binding: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<ExitCode, String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(rustdoc) = std::env::var_os("NESTRS_REAL_RUSTDOC")
        && !is_rustc_wrapper_invocation(&args)
    {
        return run_rustdoc(&rustdoc, args);
    }
    if args.is_empty() || args[0].starts_with('-') {
        return Err("invoke this driver as RUSTC_WRAPPER with rustc as its first argument".into());
    }
    let rustc = args[0].clone();
    let crate_name = flag_value(&args, "--crate-name").map(str::to_owned);
    let uses_core = has_extern(&args, "nestrs_core");
    let is_probe = crate_name.is_none()
        || args
            .iter()
            .any(|arg| arg == "--print" || arg.starts_with("--print="));
    // Every downstream crate may need the bridge while decoding an upstream
    // crate's metadata, even when it does not depend on nestrs-core itself.
    let bridge = if !is_probe {
        let driver = std::env::current_exe().map_err(|error| error.to_string())?;
        let bridge = Bridge::locate(&driver)?;
        inject_dependency_search(&mut args, &bridge)?;
        Some(bridge)
    } else {
        None
    };
    if !uses_core || is_probe {
        // Cargo's version/sysroot/capability probes must retain rustc behavior.
        let status = Command::new(&rustc)
            .args(&args[1..])
            .env_remove("RUSTC_BOOTSTRAP")
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(if status.success() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    let crate_name = crate_name.expect("checked compiler crate name");
    check_toolchain(&rustc)?;
    inject_extern(
        &mut args,
        &bridge.expect("compilation units locate the bridge"),
    )?;
    if flag_value(&args, "--sysroot").is_none() {
        args.push("--sysroot".into());
        args.push(compiler_output(&rustc, &["--print", "sysroot"])?);
    }
    let output = PathBuf::from(
        std::env::var_os("NESTRS_COMPILER_OUTPUT")
            .ok_or("NESTRS_COMPILER_OUTPUT is required; invoke the build through cargo nestrs")?,
    );
    let mut identity = DefaultHasher::new();
    args.hash(&mut identity);
    let artifacts = output.join(format!("{crate_name}-{:016x}", identity.finish()));
    fs::create_dir_all(&artifacts).map_err(|error| error.to_string())?;
    for name in ["analysis.json", "compilation.json"] {
        let path = artifacts.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    let sources = Sources::default();
    let mut discover = Discover {
        analysis: None,
        sources: sources.clone(),
    };
    let first = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut discover);
    });
    if first != ExitCode::SUCCESS {
        return Ok(first);
    }
    let analysis = discover
        .analysis
        .ok_or("compiler did not reach semantic analysis")??;
    let snapshots = {
        let snapshots = sources.lock().map_err(|error| error.to_string())?;
        for (path, original) in snapshots.iter() {
            if &fs::read_to_string(path).map_err(|error| error.to_string())? != original {
                return Err(source_changed(path).to_string());
            }
        }
        for insertion in &analysis.insertions {
            let path = insertion
                .path
                .canonicalize()
                .map_err(|error| error.to_string())?;
            let original = snapshots.get(&path).ok_or_else(|| {
                format!(
                    "source was not read in the first compiler pass: {}",
                    path.display()
                )
            })?;
            if &insertion.expected_source != original {
                return Err(format!(
                    "source changed during semantic analysis: {}",
                    path.display()
                ));
            }
        }
        snapshots.clone()
    };
    let manifest = analysis_json(&crate_name, &analysis);
    fs::write(artifacts.join("analysis.json"), manifest).map_err(|error| error.to_string())?;
    let expected = (
        analysis.providers,
        analysis.requests,
        analysis.explicit_bindings + analysis.generated_bindings,
    );
    let loader = OverlayFileLoader::from_insertions(analysis.insertions, &artifacts)
        .map_err(|error| error.to_string())?;
    let mut generate = Generate {
        loader: Some(loader),
        snapshots,
        expected,
        validation: None,
    };
    let second = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut generate);
    });
    let validated = generate
        .validation
        .unwrap_or_else(|| Err("final compiler did not reach semantic verification".into()));
    fs::write(
        artifacts.join("compilation.json"),
        format!(
            "{{\"passed\":{},\"passes\":2}}\n",
            second == ExitCode::SUCCESS && validated.is_ok(),
        ),
    )
    .map_err(|error| error.to_string())?;
    if second == ExitCode::SUCCESS {
        validated?;
    }
    Ok(second)
}

fn is_rustc_wrapper_invocation(args: &[String]) -> bool {
    args.first().is_some_and(|first| {
        Path::new(first)
            .file_name()
            .is_some_and(|name| name == "rustc" || name == "rustc.exe")
    })
}

/// rustdoc bypasses RUSTC_WRAPPER, so inject the same ordinary proc-macro extern
/// before delegating to the pinned, unmodified rustdoc. This does not claim to
/// run the automatic-binding compiler passes on newly declared doctest services.
fn run_rustdoc(rustdoc: &std::ffi::OsStr, mut args: Vec<String>) -> Result<ExitCode, String> {
    let uses_core = has_extern(&args, "nestrs_core");
    if flag_value(&args, "--crate-name").is_some() || uses_core {
        let driver = std::env::current_exe().map_err(|error| error.to_string())?;
        let bridge = Bridge::locate(&driver)?;
        inject_dependency_search(&mut args, &bridge)?;
        if uses_core {
            inject_extern(&mut args, &bridge)?;
        }
    }
    let status = Command::new(rustdoc)
        .args(args)
        .env_remove("RUSTC_BOOTSTRAP")
        .status()
        .map_err(|error| format!("cannot start the pinned rustdoc: {error}"))?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find_map(|pair| (pair[0] == flag).then_some(pair[1].as_str()))
        .or_else(|| {
            args.iter()
                .find_map(|arg| arg.strip_prefix(&format!("{flag}=")))
        })
}

fn compiler_output(rustc: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(rustc)
        .args(args)
        .env_remove("RUSTC_BOOTSTRAP")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "rustc query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|error| error.to_string())
}

fn check_toolchain(rustc: &str) -> Result<(), String> {
    let version = compiler_output(rustc, &["-vV"])?;
    let expected = cargo_nestrs::toolchain::CompilerIdentity::pinned()?;
    let actual = cargo_nestrs::toolchain::CompilerIdentity::parse(&version)?;
    expected.verify(&actual)
}

fn analysis_json(crate_name: &str, analysis: &Analysis) -> String {
    let mut bindings = Vec::new();
    for insertion in &analysis.insertions {
        for binding in &insertion.bindings {
            bindings.push(format!(
                "{{\"concrete\":{},\"interface\":{},\"source\":{},\"line\":{},\"column\":{},\"insertion_file\":{},\"insertion_offset\":{}}}",
                json(&binding.concrete), json(&binding.interface), json(&binding.source_file),
                binding.source_line, binding.source_column, json(&insertion.path.to_string_lossy()), insertion.offset,
            ));
        }
    }
    format!(
        "{{\"crate\":{},\"providers\":{},\"requests\":{},\"generated_bindings\":{},\"explicit_bindings\":{},\"bindings\":[{}]}}\n",
        json(crate_name),
        analysis.providers,
        analysis.requests,
        analysis.generated_bindings,
        analysis.explicit_bindings,
        bindings.join(","),
    )
}

fn json(value: &str) -> String {
    let mut encoded = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            c if c <= '\u{1f}' => write!(&mut encoded, "\\u{:04x}", c as u32).unwrap(),
            c => encoded.push(c),
        }
    }
    encoded.push('"');
    encoded
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn rustdoc_arguments_are_not_mistaken_for_cargo_rustc_wrapper_calls() {
        for first in ["rustc", "/toolchain/bin/rustc", "C:/toolchain/rustc.exe"] {
            assert!(is_rustc_wrapper_invocation(&[
                first.into(),
                "--crate-name".into()
            ]));
        }
        for first in ["--edition=2024", "--crate-name", "src/lib.rs", "rustc.rs"] {
            assert!(!is_rustc_wrapper_invocation(&[first.into()]));
        }
        assert!(!is_rustc_wrapper_invocation(&[]));
    }

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "nestrs-source-snapshot-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn repeated_reads_cannot_replace_the_original_snapshot() {
        let directory = Directory::new();
        let path = directory.0.join("module.rs");
        fs::write(&path, "pub struct Original;").unwrap();
        let sources = Sources::default();
        let loader = SnapshotLoader(sources.clone());
        assert_eq!(loader.read_file(&path).unwrap(), "pub struct Original;");
        fs::write(&path, "pub struct Changed;").unwrap();
        assert_eq!(
            loader.read_file(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            sources.lock().unwrap()[&path.canonicalize().unwrap()],
            "pub struct Original;"
        );
    }

    #[test]
    fn second_pass_checks_unmodified_modules_too_and_rejects_new_sources() {
        let directory = Directory::new();
        let path = directory.0.join("without_generated_bindings.rs");
        fs::write(&path, "pub struct Original;").unwrap();
        let sources = Sources::default();
        SnapshotLoader(sources.clone()).read_file(&path).unwrap();
        let loader = CheckedLoader {
            loader: OverlayFileLoader::from_insertions(vec![], &directory.0.join("artifacts"))
                .unwrap(),
            snapshots: sources.lock().unwrap().clone(),
        };
        assert_eq!(loader.read_file(&path).unwrap(), "pub struct Original;");
        fs::write(&path, "pub struct Changed;").unwrap();
        assert_eq!(
            loader.read_file(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let new_path = directory.0.join("unexpected.rs");
        fs::write(&new_path, "pub struct Unexpected;").unwrap();
        assert_eq!(
            loader.read_file(&new_path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
