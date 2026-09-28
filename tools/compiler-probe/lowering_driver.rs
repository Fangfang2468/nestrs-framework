//! Probe the actual pre-typecheck integration boundary of the pinned rustc.
//!
//! Registering a tool namespace makes attributes legal but does not expand
//! them. This probe only observes ASTs: it deliberately does not rewrite user
//! code, replace rustc's expansion pipeline, or claim to lower DI declarations.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_ast_pretty;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_resolve;
extern crate rustc_span;

use rustc_ast::{
    ast,
    visit::{self, Visitor},
};
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_span::{Ident, Symbol, source_map::SourceMap};
use std::fmt::Write as _;

struct Observe<'a> {
    phase: &'static str,
    source_map: &'a SourceMap,
}

impl<'ast> Visitor<'ast> for Observe<'_> {
    fn visit_item(&mut self, item: &'ast ast::Item) {
        if matches!(item.kind, ast::ItemKind::Struct(..)) {
            let name = item.kind.ident().expect("struct has an identifier");
            println!(
                "{{\"phase\":{},\"kind\":\"struct\",\"name\":{},\"source\":{},\"from_expansion\":{},\"declaration\":{}}}",
                json(self.phase),
                json(name.as_str()),
                json(&self.source_map.span_to_diagnostic_string(item.span)),
                item.span.from_expansion(),
                json(&rustc_ast_pretty::pprust::item_to_string(item)),
            );
        }
        visit::walk_item(self, item);
    }
}

struct LoweringProbe;

impl Callbacks for LoweringProbe {
    fn config(&mut self, config: &mut interface::Config) {
        config.override_queries = Some(|_, providers| {
            providers.queries.registered_tools = |tcx, ()| {
                let (_, attrs) = &*tcx.crate_for_resolver(()).borrow();
                // Retain rustc's normal explicitly registered and built-in
                // tools. The only addition is this experiment's namespace.
                let mut tools = rustc_resolve::registered_tools_ast(tcx.dcx(), attrs, tcx.sess);
                tools.insert(Ident::with_dummy_span(Symbol::intern("nestrs")));
                tools
            };
        });
    }

    fn after_crate_root_parsing(
        &mut self,
        compiler: &interface::Compiler,
        krate: &mut ast::Crate,
    ) -> Compilation {
        Observe {
            phase: "root_parsed",
            source_map: compiler.sess.source_map(),
        }
        .visit_crate(krate);
        Compilation::Continue
    }

    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let (_, krate) = tcx.resolver_for_lowering();
        Observe {
            phase: "expanded",
            source_map: tcx.sess.source_map(),
        }
        .visit_crate(&krate.borrow());
        Compilation::Continue
    }

    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        _tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        println!("{{\"phase\":\"analyzed\",\"kind\":\"complete\"}}");
        Compilation::Continue
    }
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

fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().collect();
    rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut LoweringProbe);
    })
}
