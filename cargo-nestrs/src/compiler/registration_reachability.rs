//! 保留下游反射计划需要的类型化 adapter 与其真实代码依赖。
//!
//! Encoding a private callback's MIR alone is insufficient: downstream codegen
//! can instantiate that MIR, but cannot instantiate an upstream private static
//! referenced by it. Seed rustc's normal reachability analysis with authenticated
//! typed adapter callbacks so its existing inline/const/generic traversal retains
//! the complete necessary symbol closure in the producing crate.
//! The runtime's public items inside private modules are also codegen inputs:
//! retain their bodies once in core instead of giving every provider rlib its
//! own strong definition of the same nongeneric runtime function.
//!
//! This only changes the least-direct codegen reachability level. Declaration
//! visibility, source name resolution and Rust reexports remain private.

use rustc_hir::def_id::LOCAL_CRATE;
use rustc_middle::{
    middle::privacy::{EffectiveVisibilities, EffectiveVisibility, Level},
    ty::{TyCtxt, Visibility},
    util::Providers,
};
use std::sync::OnceLock;

type EffectiveQuery = for<'tcx> fn(TyCtxt<'tcx>, ()) -> &'tcx EffectiveVisibilities;
static ORIGINAL_EFFECTIVE: OnceLock<EffectiveQuery> = OnceLock::new();

pub fn provide(providers: &mut Providers) {
    ORIGINAL_EFFECTIVE.get_or_init(|| providers.queries.effective_visibilities);
    providers.queries.effective_visibilities = declaration_reachability;
}

fn declaration_reachability(tcx: TyCtxt<'_>, (): ()) -> &EffectiveVisibilities {
    let original =
        ORIGINAL_EFFECTIVE
            .get()
            .expect("Nestrs declaration reachability provider was installed")(tcx, ());
    let mut effective = original.clone();
    let runtime = tcx.crate_name(LOCAL_CRATE).as_str() == "nestrs_core"
        && !tcx
            .crates(())
            .iter()
            .any(|&krate| tcx.crate_name(krate).as_str() == "nestrs_core");
    for definition in tcx.hir_body_owners() {
        let runtime_entry = runtime
            && tcx.def_kind(definition).has_codegen_attrs()
            && tcx.visibility(definition).is_public();
        if !runtime_entry
            && !super::registration_codegen::authenticated_callback(tcx, definition.to_def_id())
        {
            continue;
        }
        effective.update(
            definition,
            None,
            Visibility::Restricted(tcx.parent_module_from_def_id(definition).into()),
            EffectiveVisibility::from_vis(Visibility::Public),
            Level::ReachableThroughImplTrait,
            tcx,
        );
    }
    effective.check_invariants(tcx);
    tcx.arena.alloc(effective)
}
