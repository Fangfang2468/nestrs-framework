//! A version-pinned semantic probe, not a production Nestrs compiler or CLI.
//!
//! This driver reads rustc's analyzed type context. It deliberately does not
//! scan source text, synthesize bindings, invoke application code, or emit an
//! application binary. Every stdout line is a JSON object; diagnostics remain
//! on stderr. The matching toolchain and verifier are documented alongside it.

#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;

use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def::DefKind;
use rustc_interface::interface;
use rustc_middle::ty::{self, TyCtxt};
use std::fmt::Write as _;

struct SemanticProbe;

impl Callbacks for SemanticProbe {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let source_map = tcx.sess.source_map();
        let mut records = Vec::new();

        // This includes definitions produced by macro expansion, regardless of
        // module visibility. `DefId` and `Ty` come from semantic analysis.
        for def_id in tcx.iter_local_def_id() {
            let span = tcx.def_span(def_id);
            let path = tcx.def_path_str(def_id);
            let source = source_map.span_to_diagnostic_string(span);
            match tcx.def_kind(def_id) {
                DefKind::Impl { of_trait: true } => {
                    // Open generic impls remain symbolic; rendering their
                    // resolved identities does not claim a closed instance.
                    let trait_ref = tcx
                        .impl_trait_ref(def_id)
                        .instantiate_identity()
                        .skip_normalization();
                    let self_type = tcx
                        .type_of(def_id)
                        .instantiate_identity()
                        .skip_normalization();
                    records.push(format!(
                        "{{\"kind\":\"impl\",\"trait_path\":{},\"trait_ref\":{},\"self_type\":{},\"impl_path\":{},\"source\":{},\"from_expansion\":{},\"generic\":{},\"polarity\":{}}}",
                        json(&tcx.def_path_str(trait_ref.def_id)),
                        json(&trait_ref.to_string()),
                        json(&self_type.to_string()),
                        json(&path),
                        json(&source),
                        span.from_expansion(),
                        tcx.generics_of(def_id).count() != 0,
                        json(&format!("{:?}", tcx.impl_polarity(def_id))),
                    ));
                }
                DefKind::TyAlias => {
                    let resolved = tcx.type_of(def_id).instantiate_identity();
                    let normalized = tcx.try_normalize_erasing_regions(
                        ty::TypingEnv::post_analysis(tcx, def_id),
                        resolved,
                    );
                    let normalized = match normalized {
                        Ok(value) => value.to_string(),
                        Err(error) => format!("<normalization failed: {error:?}>"),
                    };
                    records.push(format!(
                        "{{\"kind\":\"type_alias\",\"path\":{},\"resolved_type\":{},\"normalized_type\":{},\"source\":{},\"generic\":{}}}",
                        json(&path),
                        json(&resolved.skip_normalization().to_string()),
                        json(&normalized),
                        json(&source),
                        tcx.generics_of(def_id).count() != 0,
                    ));
                }
                _ => {}
            }
        }
        records.sort_unstable();
        for record in records {
            println!("{record}");
        }
        Compilation::Stop
    }
}

/// Escape arbitrary compiler display strings without assuming Rust's Debug
/// escapes are JSON-compatible (for example, JSON has no `\u{...}` syntax).
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

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().collect();
    rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut SemanticProbe);
    })
}
