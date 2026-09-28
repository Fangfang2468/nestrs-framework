//! A graph build retains the application's declarations and query roots while
//! replacing its direct binary entry point. It never calls the business main.

use rustc_ast::tokenstream::{AttrTokenStream, AttrTokenTree, LazyAttrTokenStream, TokenTree};
use rustc_ast::{self as ast, token};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_span::{FileName, Ident, Symbol};
use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
};

struct GraphTarget {
    crate_name: String,
    binary: OsString,
    manifest: PathBuf,
    source: PathBuf,
    proof: PathBuf,
}

impl GraphTarget {
    fn from_environment() -> Result<Option<Self>, String> {
        let Some(crate_name) = std::env::var_os("NESTRS_GRAPH_TARGET") else {
            return Ok(None);
        };
        let required = |name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!("cargo nestrs graph requires {name}; refusing an unverified entry")
                })
        };
        let canonical = |name| -> Result<PathBuf, String> {
            PathBuf::from(required(name)?)
                .canonicalize()
                .map_err(|error| format!("cannot identify graph {name}: {error}"))
        };
        Ok(Some(Self {
            crate_name: crate_name
                .into_string()
                .map_err(|_| "graph crate name must be UTF-8")?,
            binary: required("NESTRS_GRAPH_BINARY")?,
            manifest: canonical("NESTRS_GRAPH_MANIFEST")?,
            source: canonical("NESTRS_GRAPH_SOURCE")?,
            proof: PathBuf::from(required("NESTRS_GRAPH_PROOF")?),
        }))
    }

    fn matches(
        &self,
        crate_name: &str,
        source: &Path,
        cargo_binary: Option<&OsStr>,
        cargo_manifest: Option<&OsStr>,
    ) -> Result<bool, String> {
        if source.canonicalize().ok().as_ref() != Some(&self.source) {
            return Ok(false);
        }
        let manifest = cargo_manifest.and_then(|path| Path::new(path).canonicalize().ok());
        let binary_crate = self.binary.to_str().map(|name| name.replace('-', "_"));
        if crate_name != self.crate_name
            || binary_crate.as_deref() != Some(crate_name)
            || cargo_binary != Some(self.binary.as_os_str())
            || manifest.as_ref() != Some(&self.manifest)
        {
            return Err("cargo nestrs graph could not verify the selected binary name and package; refusing to compile an unmodified application entry".into());
        }
        Ok(true)
    }

    fn matches_environment(&self, crate_name: &str, source: &Path) -> Result<bool, String> {
        self.matches(
            crate_name,
            source,
            std::env::var_os("CARGO_BIN_NAME").as_deref(),
            std::env::var_os("CARGO_MANIFEST_DIR").as_deref(),
        )
    }
}

/// Match a real source file as well as Cargo's unnormalized binary/package
/// identity. A same-named dependency or build script is not the selected entry.
pub fn matches_target(crate_name: &str, source: &Path) -> Result<bool, String> {
    GraphTarget::from_environment()?
        .map(|target| target.matches_environment(crate_name, source))
        .unwrap_or(Ok(false))
}

/// Cargo passes the source as a standalone rustc argument; it need not have a
/// `.rs` suffix. Compare actual paths instead of guessing from that suffix.
pub fn matches_arguments(crate_name: &str, args: &[String]) -> Result<bool, String> {
    let Some(target) = GraphTarget::from_environment()? else {
        return Ok(false);
    };
    for arg in args.iter().skip(1) {
        let source = Path::new(arg);
        if source.canonicalize().ok().as_ref() == Some(&target.source) {
            return target.matches_environment(crate_name, source);
        }
    }
    Ok(false)
}

/// An accepted rebuild invalidates an earlier proof. A fresh Cargo cache hit
/// does not invoke the driver and retains its proof in the isolated graph cache.
pub fn invalidate_proof() -> Result<(), String> {
    if let Some(target) = GraphTarget::from_environment()? {
        match fs::remove_file(&target.proof) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot invalidate graph entry proof: {error}")),
        }
    }
    Ok(())
}

/// Only call after both compiler passes, entry validation and final compilation
/// have succeeded. The CLI checks this proof before executing the artifact.
pub fn write_proof() -> Result<(), String> {
    let target = GraphTarget::from_environment()?
        .ok_or("missing graph selection while writing its entry proof")?;
    if !target.matches_environment(&target.crate_name, &target.source)? {
        return Err("graph source changed before its entry proof could be written".into());
    }
    let parent = target
        .proof
        .parent()
        .ok_or("graph entry proof has no parent")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let bytes = serde_json::to_vec(&serde_json::json!({
        "binary": target.binary.to_str().ok_or("graph binary name must be UTF-8")?,
        "manifest": target.manifest,
        "crate": target.crate_name,
        "source": target.source,
    }))
    .map_err(|error| error.to_string())?;
    let temporary = target
        .proof
        .with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&temporary, bytes)
        .and_then(|()| fs::rename(&temporary, &target.proof))
        .map_err(|error| format!("cannot write graph entry proof: {error}"))
}

fn matches_session(session: &rustc_session::Session, crate_name: &str) -> bool {
    let rustc_session::config::Input::File(source) = &session.io.input else {
        return false;
    };
    matches_target(crate_name, source).unwrap_or_else(|error| session.dcx().fatal(error))
}

/// Check the actual entry after cfg/attribute expansion as well. In particular,
/// cfg_attr can introduce no_main after the initial AST was inspected.
pub fn validate(tcx: TyCtxt<'_>) {
    if !tcx
        .crate_types()
        .contains(&rustc_session::config::CrateType::Executable)
        || !matches_session(
            tcx.sess,
            tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE).as_str(),
        )
    {
        return;
    }
    let generated_entry = tcx.entry_fn(()).is_some_and(|(entry, _)| {
        let source = tcx
            .sess
            .source_map()
            .lookup_source_file(tcx.def_span(entry).lo());
        matches!(&source.name, FileName::Custom(name) if name == "nestrs graph entry")
    });
    if !generated_entry {
        tcx.dcx().fatal("cargo nestrs graph requires the generated standard Rust entry; #![no_main] and custom process entries are not supported");
    }
}

pub fn prepare(compiler: &interface::Compiler, krate: &mut ast::Crate) {
    if !compiler
        .sess
        .opts
        .crate_types
        .contains(&rustc_session::config::CrateType::Executable)
    {
        return;
    }
    let Some(name) = compiler.sess.opts.crate_name.as_ref() else {
        return;
    };
    if !matches_session(&compiler.sess, name.as_str()) {
        return;
    }
    if krate
        .attrs
        .iter()
        .any(|attr| attr.has_name(rustc_span::sym::no_main))
    {
        compiler.sess.dcx().fatal(
            "cargo nestrs graph does not support #![no_main]; a standard Rust binary entry is required",
        );
    }
    let mut count = 0;
    for item in &mut krate.items {
        if let ast::ItemKind::Fn(function) = &mut item.kind
            && function.ident.name.as_str() == "main"
        {
            let old = function.ident;
            function.ident = Ident::new(
                Symbol::intern(&format!("__nestrs_graph_original_main_{count}")),
                function.ident.span,
            );
            if let Some(tokens) = &item.tokens {
                item.tokens = Some(LazyAttrTokenStream::new_direct(AttrTokenStream::new(
                    rename_tokens(
                        tokens.to_attr_token_stream().to_token_trees(),
                        old,
                        function.ident,
                    ),
                )));
            }
            count += 1;
        }
    }
    if count == 0 {
        compiler.sess.dcx().fatal(
            "cargo nestrs graph currently requires a directly declared binary main; macro-generated entry points are not yet supported",
        );
    }
    let source = r#"
        #[allow(dead_code)]
        fn main() {
            match ::nestrs_core::__private::dependency_graph_json() {
                Ok(data) => println!("{}", data),
                Err(error) => { eprintln!("{}", error); ::std::process::exit(1); }
            }
        }
    "#;
    let mut parser = rustc_parse::new_parser_from_source_str(
        &compiler.sess.psess,
        FileName::Custom("nestrs graph entry".into()),
        source.into(),
        rustc_parse::lexer::StripTokens::Nothing,
    )
    .unwrap_or_else(|diagnostics| {
        for diagnostic in diagnostics {
            diagnostic.emit();
        }
        compiler
            .sess
            .dcx()
            .fatal("cannot parse the Nestrs graph entry")
    });
    while parser.token != token::Eof {
        match parser.parse_item(
            rustc_parse::parser::ForceCollect::No,
            rustc_parse::parser::AllowConstBlockItems::No,
        ) {
            Ok(Some(item)) => {
                for original in &mut krate.items {
                    if let ast::ItemKind::Fn(function) = &original.kind
                        && function
                            .ident
                            .name
                            .as_str()
                            .starts_with("__nestrs_graph_original_main_")
                    {
                        original.attrs.extend(item.attrs.iter().cloned());
                    }
                }
                krate.items.push(item);
            }
            Ok(None) => break,
            Err(error) => {
                error.emit();
                break;
            }
        }
    }
}

fn rename_tokens(
    tokens: Vec<TokenTree>,
    original: Ident,
    replacement: Ident,
) -> Vec<AttrTokenTree> {
    tokens
        .into_iter()
        .map(|tree| match tree {
            TokenTree::Token(mut token, spacing) => {
                if token.span == original.span
                    && let token::Ident(name, _) = &mut token.kind
                    && *name == original.name
                {
                    *name = replacement.name;
                }
                AttrTokenTree::Token(token, spacing)
            }
            TokenTree::Delimited(span, spacing, delimiter, stream) => AttrTokenTree::Delimited(
                span,
                spacing,
                delimiter,
                AttrTokenStream::new(rename_tokens(
                    stream.iter().cloned().collect(),
                    original,
                    replacement,
                )),
            ),
        })
        .collect()
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "nestrs graph identity {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(directory.join("first")).unwrap();
            fs::create_dir_all(directory.join("second")).unwrap();
            for source in ["first/entry", "first/build.rs", "second/entry"] {
                fs::write(directory.join(source), "fn main() {}\n").unwrap();
            }
            Self(directory)
        }

        fn target(&self, binary: &str) -> GraphTarget {
            GraphTarget {
                crate_name: binary.replace('-', "_"),
                binary: binary.into(),
                manifest: self.0.join("first").canonicalize().unwrap(),
                source: self.0.join("first/entry").canonicalize().unwrap(),
                proof: self.0.join("proof.json"),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn graph_identity_requires_the_raw_binary_and_canonical_package() {
        let fixture = Fixture::new();
        let target = fixture.target("my-app");
        let source = fixture.0.join("first/../first/entry");
        let manifest = fixture.0.join("first/.");
        assert!(
            target
                .matches(
                    "my_app",
                    &source,
                    Some(OsStr::new("my-app")),
                    Some(manifest.as_os_str()),
                )
                .unwrap()
        );
        for binary in [None, Some(OsStr::new("my_app"))] {
            assert!(
                target
                    .matches("my_app", &source, binary, Some(manifest.as_os_str()))
                    .is_err()
            );
        }
        assert!(
            target
                .matches(
                    "different",
                    &source,
                    Some(OsStr::new("my-app")),
                    Some(manifest.as_os_str())
                )
                .is_err()
        );
    }

    #[test]
    fn inherited_binary_name_cannot_select_a_same_named_build_script() {
        let fixture = Fixture::new();
        let target = fixture.target("build-script-build");
        assert!(
            !target
                .matches(
                    "build_script_build",
                    &fixture.0.join("first/build.rs"),
                    Some(OsStr::new("build-script-build")),
                    Some(target.manifest.as_os_str()),
                )
                .unwrap()
        );
    }

    #[test]
    fn same_binary_name_in_another_package_never_selects_its_source() {
        let fixture = Fixture::new();
        let target = fixture.target("app");
        let other_manifest = fixture.0.join("second");
        assert!(
            !target
                .matches(
                    "app",
                    &fixture.0.join("second/entry"),
                    Some(OsStr::new("app")),
                    Some(other_manifest.as_os_str()),
                )
                .unwrap()
        );
        // If Cargo claims our exact input but a different package, a stale
        // proof must not permit an ordinary rebuilt executable to be launched.
        assert!(
            target
                .matches(
                    "app",
                    &target.source,
                    Some(OsStr::new("app")),
                    Some(other_manifest.as_os_str()),
                )
                .is_err()
        );
    }
}
