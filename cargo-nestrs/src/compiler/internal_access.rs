//! Compiler-only access to the runtime's private implementation modules.
//!
//! The metadata and ordinary rustc visibility of nestrs-core are unchanged. The
//! wrapper temporarily exposes its external names to resolution, then rejects
//! every access not originating in an authenticated framework macro expansion
//! or an exact compiler-generated source range. This is deliberately paired
//! with a HIR audit; installing the resolver hooks without running `validate`
//! is not a supported compilation mode.

extern crate rustc_abi;

use rustc_abi::ExternAbi;
use rustc_hir::{
    self as hir,
    def::Res,
    def_id::{CrateNum, DefId, LOCAL_CRATE, LocalDefId},
    intravisit,
};
use rustc_middle::{
    metadata::ModChild,
    ty::{self, TyCtxt},
    util::Providers,
};
use rustc_span::{
    Span, Symbol,
    hygiene::{ExpnData, ExpnKind, LocalExpnId, MacroKind, Transparency},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

type VisibilityQuery = for<'tcx> fn(TyCtxt<'tcx>, DefId) -> ty::Visibility<DefId>;
type ChildrenQuery = for<'tcx> fn(TyCtxt<'tcx>, DefId) -> &'tcx [ModChild];
type ResolutionsQuery = for<'tcx> fn(TyCtxt<'tcx>, ()) -> &'tcx ty::ResolverGlobalCtxt;
type SpanQuery = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Span;
type IdentSpanQuery = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<Span>;
static VISIBILITY: OnceLock<VisibilityQuery> = OnceLock::new();
static CHILDREN: OnceLock<ChildrenQuery> = OnceLock::new();
static RESOLUTIONS: OnceLock<ResolutionsQuery> = OnceLock::new();
static DEF_SPAN: OnceLock<SpanQuery> = OnceLock::new();
static DEF_IDENT_SPAN: OnceLock<IdentSpanQuery> = OnceLock::new();
static TRUSTED_RANGES: Mutex<Vec<TrustedRange>> = Mutex::new(Vec::new());
static BRIDGE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The CLI-selected bridge artifact, including for transitive-only consumers.
/// A different dependency with the same crate name is not this compiler tool.
pub fn set_bridge_path(path: PathBuf) {
    *BRIDGE_PATH.lock().expect("bridge identity lock poisoned") = Some(path);
}

fn matches_extern(tcx: TyCtxt<'_>, krate: CrateNum, name: &str) -> bool {
    tcx.sess.opts.externs.get(name).is_some_and(|entry| {
        entry.files().is_some_and(|files| {
            let loaded = tcx.used_crate_source(krate);
            let paths: Vec<_> = loaded
                .paths()
                .filter_map(|path| path.canonicalize().ok())
                .collect();
            files
                .into_iter()
                .any(|file| paths.iter().any(|path| path == file.canonicalized()))
        })
    })
}

// This predicate is also called during name resolution. Inspect the supplied
// crate only: enumerating tcx.crates() here would freeze the dependency store
// before rustc has finished loading the remaining externs.
fn runtime_crate(tcx: TyCtxt<'_>, krate: CrateNum) -> bool {
    if tcx.crate_name(krate).as_str() != "nestrs_core" {
        return false;
    }
    if krate == LOCAL_CRATE {
        return tcx.sess.opts.externs.get("nestrs_core").is_none();
    }
    if tcx.sess.opts.externs.get("nestrs_core").is_some() {
        return matches_extern(tcx, krate, "nestrs_core");
    }
    // Transitive-only consumers have no direct extern alias. The registry
    // collector separately requires one compatible runtime identity.
    true
}

fn tool_bridge(tcx: TyCtxt<'_>, krate: CrateNum) -> bool {
    if tcx.crate_name(krate).as_str() != "nestrs_tool_bridge" {
        return false;
    }
    if let Some(path) = BRIDGE_PATH
        .lock()
        .expect("bridge identity lock poisoned")
        .as_ref()
    {
        return tcx
            .used_crate_source(krate)
            .paths()
            .any(|loaded| loaded.canonicalize().is_ok_and(|loaded| &loaded == path));
    }
    // Standalone query-hook regression harnesses select their bridge explicitly.
    matches_extern(tcx, krate, "nestrs")
}

/// UTF-8 byte offsets in the exact virtual source supplied to rustc. Callers
/// calculate these while applying insertions, never from user-written markers.
#[derive(Clone, Debug)]
pub struct TrustedRange {
    pub path: PathBuf,
    pub start: usize,
    pub end: usize,
}

/// Replace the ranges for each compiler invocation, including empty discovery
/// passes, so permissions cannot leak from a previous compilation.
pub fn set_trusted_ranges(ranges: Vec<TrustedRange>) {
    *TRUSTED_RANGES
        .lock()
        .expect("internal source range lock poisoned") = ranges;
}

pub fn install_queries(providers: &mut Providers) {
    let _ = VISIBILITY.set(providers.extern_queries.visibility);
    let _ = CHILDREN.set(providers.extern_queries.module_children);
    let _ = RESOLUTIONS.set(providers.queries.resolutions);
    let _ = DEF_SPAN.set(providers.queries.def_span);
    let _ = DEF_IDENT_SPAN.set(providers.queries.def_ident_span);
    providers.extern_queries.visibility = visibility;
    providers.extern_queries.module_children = module_children;
    providers.queries.resolutions = resolutions;
    providers.queries.def_span = definition_span;
    providers.queries.def_ident_span = definition_ident_span;
}

fn definition_span(tcx: TyCtxt<'_>, definition: LocalDefId) -> Span {
    let original = (DEF_SPAN.get().expect("Nestrs span hook not installed"))(tcx, definition);
    mark_generated(tcx, original)
}

fn definition_ident_span(tcx: TyCtxt<'_>, definition: LocalDefId) -> Option<Span> {
    let original = (DEF_IDENT_SPAN
        .get()
        .expect("Nestrs span hook not installed"))(tcx, definition);
    original.map(|span| mark_generated(tcx, span))
}

/// Authenticate generated source definitions in ordinary encoded span hygiene,
/// so a downstream consumer does not have to trust a callback name or a source
/// filename. This leaves the HIR's original resolution contexts untouched.
fn mark_generated(tcx: TyCtxt<'_>, span: Span) -> Span {
    if span.from_expansion() || !trusted_span(tcx, span) {
        return span;
    }
    let anchor = tcx.crates(()).iter().find_map(|&krate| {
        if tcx.crate_name(krate).as_str() != "nestrs_core" {
            return None;
        }
        // plan 是真实私有运行期模块，其 pub(crate) 子模块不保证出现在外部名字表。
        // 按已保留 MIR 的 DefId 找执行 ABI，避免把“可从源码命名”误当成“有真实定义”。
        // 先查 MIR 可用性跳过 metadata 表中的空洞，再核对完整身份与签名。
        (0..tcx.num_extern_def_ids(krate)).find_map(|index| {
            let definition = DefId {
                krate,
                index: rustc_hir::def_id::DefIndex::from_usize(index),
            };
            (tcx.is_mir_available(definition)
                && tcx
                    .opt_item_name(definition)
                    .is_some_and(|name| name.as_str() == crate::protocol::PlanSink::Options.name())
                && generated_anchor(tcx, definition))
            .then_some(definition)
        })
    });
    let Some(anchor) = anchor else {
        return span;
    };
    let data = ExpnData::default(
        ExpnKind::Macro(
            MacroKind::Bang,
            Symbol::intern("__nestrs_compiler_generated"),
        ),
        span,
        tcx.sess.edition(),
        Some(anchor),
        None,
    );
    let expansion = tcx.with_stable_hashing_context(|context| LocalExpnId::fresh(data, context));
    // Provenance must not alter downstream name resolution of this item.
    span.apply_mark(expansion.to_expn_id(), Transparency::Transparent)
}

fn generated_anchor(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    // 认证锚点复用真实运行期配置接合 ABI。core 不再为编译器保存空 marker，且
    // 不能只信函数拼写：必须来自匹配的 runtime crate，并核对完整普通 Rust 签名。
    // 身份比较使用真实 DefPath。def_path_str 是诊断路径，可能因 facade 的私有 use
    // 别名显示为 facade::plan，不能把用于展示的路径当成元数据中的定义身份。
    if !is_runtime(tcx, definition)
        || !tcx
            .def_path(definition)
            .data
            .iter()
            .map(|component| component.data.get_opt_name())
            .eq(["graph", "plan", crate::protocol::PlanSink::Options.name()]
                .map(|name| Some(Symbol::intern(name))))
        || tcx.def_kind(definition) != hir::def::DefKind::Fn
        || tcx.is_foreign_item(definition)
        || tcx.generics_of(definition).count() != 0
    {
        return false;
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    signature.abi() == ExternAbi::Rust
        && !signature.safety().is_safe()
        && !signature.c_variadic()
        && signature.output() == tcx.types.unit
        && matches!(signature.inputs(), [pointer, eager, concurrency]
            if matches!(pointer.kind(), ty::RawPtr(element, mutability)
                if *element == tcx.types.unit && mutability.is_mut())
                && *eager == tcx.types.bool && *concurrency == tcx.types.usize)
}

fn is_runtime(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    !definition.is_local() && runtime_crate(tcx, definition.krate)
}

fn visibility(tcx: TyCtxt<'_>, definition: DefId) -> ty::Visibility<DefId> {
    if is_runtime(tcx, definition) {
        ty::Visibility::Public
    } else {
        original_visibility(tcx, definition)
    }
}

fn original_visibility(tcx: TyCtxt<'_>, definition: DefId) -> ty::Visibility<DefId> {
    (VISIBILITY
        .get()
        .expect("Nestrs visibility hook not installed"))(tcx, definition)
}

fn module_children(tcx: TyCtxt<'_>, definition: DefId) -> &[ModChild] {
    let children = (CHILDREN.get().expect("Nestrs child hook not installed"))(tcx, definition);
    if !is_runtime(tcx, definition) {
        return children;
    }
    tcx.arena
        .alloc_from_iter(children.iter().map(|child| ModChild {
            ident: child.ident,
            res: child.res,
            vis: ty::Visibility::Public,
            reexport_chain: child.reexport_chain.clone(),
        }))
}

fn copy_child(child: &ModChild) -> ModChild {
    ModChild {
        ident: child.ident,
        res: child.res,
        vis: child.vis,
        reexport_chain: child.reexport_chain.clone(),
    }
}

/// Restore the ordinary public export surface before consumer metadata is
/// encoded. In particular, `pub use nestrs_core::*` must not reexport names
/// temporarily made resolvable for generated code.
fn resolutions(tcx: TyCtxt<'_>, (): ()) -> &ty::ResolverGlobalCtxt {
    let original = (RESOLUTIONS
        .get()
        .expect("Nestrs resolver hook not installed"))(tcx, ());
    let audit = Audit::new(tcx);
    let retained = |child: &ModChild| !child.res.opt_def_id().is_some_and(|id| audit.internal(id));
    let filtered = ty::ResolverGlobalCtxt {
        visibilities_for_hashing: original.visibilities_for_hashing.clone(),
        expn_that_defined: original.expn_that_defined.clone(),
        effective_visibilities: original.effective_visibilities.clone(),
        macro_reachable_adts: original.macro_reachable_adts.clone(),
        extern_crate_map: original.extern_crate_map.clone(),
        maybe_unused_trait_imports: original.maybe_unused_trait_imports.clone(),
        module_children: original
            .module_children
            .items()
            .map(|(id, children)| {
                (
                    *id,
                    children
                        .iter()
                        .filter(|child| retained(child))
                        .map(copy_child)
                        .collect(),
                )
            })
            .collect(),
        ambig_module_children: original
            .ambig_module_children
            .items()
            .map(|(id, children)| {
                (
                    *id,
                    children
                        .iter()
                        .filter(|child| retained(&child.main) && retained(&child.second))
                        .map(|child| rustc_middle::metadata::AmbigModChild {
                            main: copy_child(&child.main),
                            second: copy_child(&child.second),
                        })
                        .collect(),
                )
            })
            .collect(),
        glob_map: original.glob_map.clone(),
        main_def: original.main_def,
        trait_impls: original.trait_impls.clone(),
        proc_macros: original.proc_macros.clone(),
        confused_type_with_std_module: original.confused_type_with_std_module.clone(),
        doc_link_resolutions: original.doc_link_resolutions.clone(),
        doc_link_traits_in_scope: original.doc_link_traits_in_scope.clone(),
        all_macro_rules: original.all_macro_rules.clone(),
        stripped_cfg_items: original.stripped_cfg_items.clone(),
        delegation_infos: original
            .delegation_infos
            .iter()
            .map(|(id, info)| {
                (
                    *id,
                    ty::DelegationInfo {
                        resolution_id: info.resolution_id,
                    },
                )
            })
            .collect(),
    };
    tcx.arena.alloc(filtered)
}

struct Audit<'tcx> {
    tcx: TyCtxt<'tcx>,
    public: HashSet<DefId>,
    ranges: Vec<TrustedRange>,
    errors: HashSet<(DefId, Span)>,
}

impl<'tcx> Audit<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Self {
        let mut public = HashSet::new();
        for &krate in tcx.crates(()) {
            if !runtime_crate(tcx, krate) {
                continue;
            }
            let root = DefId {
                krate,
                index: rustc_hir::def_id::CRATE_DEF_INDEX,
            };
            let original = (CHILDREN.get().expect("Nestrs child hook not installed"))(tcx, root);
            public.extend(
                original
                    .iter()
                    .filter(|child| child.vis.is_public())
                    .filter_map(|child| child.res.opt_def_id()),
            );
            public.insert(root);
        }
        Self {
            tcx,
            public,
            ranges: TRUSTED_RANGES
                .lock()
                .expect("internal source range lock poisoned")
                .clone(),
            errors: HashSet::new(),
        }
    }

    fn trusted(&self, span: Span) -> bool {
        trusted_span_in(self.tcx, span, &self.ranges)
    }

    fn internal(&self, definition: DefId) -> bool {
        if definition.is_local() || !is_runtime(self.tcx, definition) {
            return false;
        }
        let mut cursor = definition;
        loop {
            if self.public.contains(&cursor) {
                return false;
            }
            if !matches!(
                self.tcx.def_kind(cursor),
                hir::def::DefKind::AssocFn
                    | hir::def::DefKind::AssocConst { .. }
                    | hir::def::DefKind::AssocTy
                    | hir::def::DefKind::Variant
                    | hir::def::DefKind::Ctor(..)
                    | hir::def::DefKind::Field
            ) || !original_visibility(self.tcx, cursor).is_public()
            {
                return true;
            }
            cursor = self.tcx.parent(cursor);
            // Inherent methods belong to an impl, whose self type may have a
            // public reexport even though its defining module is private.
            if let hir::def::DefKind::Impl { .. } = self.tcx.def_kind(cursor) {
                if matches!(
                    self.tcx.def_kind(cursor),
                    hir::def::DefKind::Impl { of_trait: true }
                ) {
                    let interface = self
                        .tcx
                        .impl_trait_ref(cursor)
                        .instantiate_identity()
                        .skip_normalization()
                        .def_id;
                    if is_runtime(self.tcx, interface) && !self.public.contains(&interface) {
                        return true;
                    }
                }
                if let ty::Adt(adt, _) = self
                    .tcx
                    .type_of(cursor)
                    .instantiate_identity()
                    .skip_normalization()
                    .kind()
                {
                    return !self.public.contains(&adt.did());
                }
                return true;
            }
        }
    }

    fn check(&mut self, definition: DefId, span: Span) {
        if self.internal(definition)
            && !self.trusted(span)
            && self.errors.insert((definition, span))
        {
            self.tcx.dcx().span_err(
                span,
                format!(
                    "Nestrs 内部实现 `{}` 只能由编译器生成代码访问；请使用公开门面 API",
                    self.tcx.def_path_str(definition),
                ),
            );
        }
    }

    fn resolution(&mut self, resolution: Res, span: Span) {
        if let Res::Def(_, definition) = resolution {
            self.check(definition, span);
        }
    }

    fn inferred_type(&mut self, value: ty::Ty<'_>, span: Span) {
        if self.trusted(span) {
            return;
        }
        for argument in value.walk() {
            let Some(value) = argument.as_type() else {
                continue;
            };
            match *value.kind() {
                ty::Adt(definition, _) => self.check(definition.did(), span),
                ty::FnDef(definition, _) => self.check(definition, span),
                ty::Dynamic(predicates, _) => {
                    if let Some(principal) = predicates.principal() {
                        self.check(principal.def_id(), span);
                    }
                }
                _ => {}
            }
        }
    }
}

/// The registry collector uses the same provenance rule as access checking.
pub fn trusted_definition(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    tcx.def_ident_span(definition)
        .is_some_and(|span| trusted_span(tcx, span))
        || trusted_span(tcx, tcx.def_span(definition))
}

pub fn trusted_span(tcx: TyCtxt<'_>, span: Span) -> bool {
    trusted_span_in(
        tcx,
        span,
        &TRUSTED_RANGES
            .lock()
            .expect("internal source range lock poisoned"),
    )
}

fn trusted_span_in(tcx: TyCtxt<'_>, span: Span, ranges: &[TrustedRange]) -> bool {
    if span.is_dummy() {
        return false;
    }
    // Standard-library macros and compiler desugarings may wrap tokens
    // emitted by our bridge. Walk through those, but stop at a user macro
    // instead of inheriting authority from its framework caller.
    let mut origin = span;
    while origin.from_expansion() {
        let expansion = origin.ctxt().outer_expn_data();
        if let Some(definition) = expansion.macro_def_id {
            if matches!(expansion.kind, ExpnKind::Macro(MacroKind::Bang, name)
                if name.as_str() == "__nestrs_compiler_generated")
                && generated_anchor(tcx, definition)
            {
                return true;
            }
            let name = tcx.crate_name(definition.krate);
            if tool_bridge(tcx, definition.krate) {
                return true;
            }
            if runtime_crate(tcx, definition.krate)
                && matches!(
                    tcx.item_name(definition).as_str(),
                    "__nestrs_query"
                        | "get_service"
                        | "get_required_service"
                        | "get_keyed_service"
                        | "get_required_keyed_service"
                )
            {
                return true;
            }
            if !matches!(name.as_str(), "core" | "alloc" | "std") {
                return false;
            }
        }
        if expansion.call_site == origin {
            return false;
        }
        origin = expansion.call_site;
    }
    let span = origin;
    let source = tcx.sess.source_map().lookup_source_file(span.lo());
    if matches!(&source.name, rustc_span::FileName::Custom(name)
        if name == "nestrs graph entry" || name == "nestrs reflection metadata")
    {
        return true;
    }
    let rustc_span::FileName::Real(name) = &source.name else {
        return false;
    };
    let Some(path) = name.local_path() else {
        return false;
    };
    let path = crate::documentation::remap_source_path(path);
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let start = source.original_relative_byte_pos(span.lo()).0 as usize;
    let end = source.original_relative_byte_pos(span.hi()).0 as usize;
    ranges
        .iter()
        .any(|range| range.path == path && start >= range.start && end <= range.end)
}

impl<'tcx> intravisit::Visitor<'tcx> for Audit<'tcx> {
    type NestedFilter = rustc_middle::hir::nested_filter::All;
    fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
        self.tcx
    }

    fn visit_path(&mut self, path: &hir::Path<'tcx>, _: hir::HirId) {
        self.resolution(path.res, path.span);
        intravisit::walk_path(self, path);
    }

    fn visit_path_segment(&mut self, segment: &hir::PathSegment<'tcx>) {
        self.resolution(segment.res, segment.ident.span);
        if let Some(arguments) = segment.args {
            self.visit_generic_args(arguments);
        }
    }

    fn visit_use(&mut self, path: &'tcx hir::UsePath<'tcx>, id: hir::HirId) {
        for resolution in [path.res.type_ns, path.res.value_ns, path.res.macro_ns]
            .into_iter()
            .flatten()
        {
            self.resolution(resolution, path.span);
        }
        intravisit::walk_use(self, path, id);
    }

    fn visit_expr(&mut self, expression: &'tcx hir::Expr<'tcx>) {
        let owner = expression.hir_id.owner.def_id;
        if !self.tcx.has_typeck_results(owner) {
            intravisit::walk_expr(self, expression);
            return;
        }
        let typeck = self.tcx.typeck(owner);
        if !trusted_definition(self.tcx, owner.to_def_id())
            && let Some(value) = typeck.node_type_opt(expression.hir_id)
        {
            self.inferred_type(value, expression.span);
        }
        if let hir::ExprKind::MethodCall(segment, ..) = expression.kind
            && let Some(definition) = typeck.type_dependent_def_id(expression.hir_id)
        {
            self.check(definition, segment.ident.span);
        }
        intravisit::walk_expr(self, expression);
    }
}

/// Run after type checking and before any application metadata/codegen output.
/// Failure must stop compilation; ordinary rustc still performs all its other
/// privacy, borrow, trait and type checks.
pub fn validate(tcx: TyCtxt<'_>) -> bool {
    let mut audit = Audit::new(tcx);
    tcx.hir_walk_toplevel_module(&mut audit);
    audit.errors.is_empty()
}
