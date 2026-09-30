//! Isolated regression harness for the production internal-access queries.
//! Its PROBE_* environment permissions are test-only and never shipped in the
//! Nestrs driver. Fixtures exercise ordinary rustc metadata across real crates.
#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

#[path = "../../cargo-nestrs/src/compiler/internal_access.rs"]
mod internal_access;

// This probe has no rustdoc carrier or source remapping. Keep the production
// access query's documentation dependency explicit without inventing a second
// path mapping implementation for ordinary source fixtures.
mod documentation {
    pub fn remap_source_path(path: &std::path::Path) -> std::path::PathBuf {
        path.to_owned()
    }
}

struct Probe;

impl rustc_driver::Callbacks for Probe {
    fn config(&mut self, config: &mut rustc_interface::interface::Config) {
        config.opts.unstable_opts.always_encode_mir = true;
        config.override_queries = Some(|_, providers| internal_access::install_queries(providers));
    }

    fn after_analysis<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> rustc_driver::Compilation {
        if let Ok(expected) = std::env::var("PROBE_EXPECT_TRUSTED") {
            let mut found = false;
            for &krate in tcx.crates(()) {
                for index in 0..tcx.num_extern_def_ids(krate) {
                    let definition = rustc_hir::def_id::DefId {
                        krate,
                        index: rustc_hir::def_id::DefIndex::from_usize(index),
                    };
                    if tcx
                        .opt_item_name(definition)
                        .is_some_and(|name| name.as_str() == "__nestrs_probe_callback")
                    {
                        assert_eq!(
                            internal_access::trusted_definition(tcx, definition),
                            expected == "yes",
                            "callback provenance must come from the compiler, never its name",
                        );
                        found = true;
                    }
                }
            }
            assert!(found, "the fixture must expose a callback to inspect");
        }
        if internal_access::validate(tcx) {
            rustc_driver::Compilation::Continue
        } else {
            rustc_driver::Compilation::Stop
        }
    }
}

fn main() -> std::process::ExitCode {
    if let Some(path) = std::env::var_os("PROBE_BRIDGE_PATH") {
        internal_access::set_bridge_path(std::path::PathBuf::from(path).canonicalize().unwrap());
    }
    if let Some(path) = std::env::var_os("PROBE_TRUSTED_FILE") {
        let path = std::path::PathBuf::from(path).canonicalize().unwrap();
        let end = std::fs::metadata(&path).unwrap().len() as usize;
        internal_access::set_trusted_ranges(vec![internal_access::TrustedRange {
            path,
            start: 0,
            end,
        }]);
    }
    let args: Vec<_> = std::env::args().collect();
    rustc_driver::catch_with_exit_code(|| rustc_driver::run_compiler(&args, &mut Probe))
}
