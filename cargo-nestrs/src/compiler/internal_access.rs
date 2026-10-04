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
    def_id::{CrateNum, DefId, LOCAL_CRATE, LocalDefId, ModId},
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

/// 外部定义原始可见性查询的函数签名。
type VisibilityQuery = for<'tcx> fn(TyCtxt<'tcx>, DefId) -> ty::Visibility<ModId>;

/// 外部模块子项查询的函数签名。
type ChildrenQuery = for<'tcx> fn(TyCtxt<'tcx>, DefId) -> &'tcx [ModChild];

/// 整个 crate 名称解析结果查询的函数签名。
type ResolutionsQuery = for<'tcx> fn(TyCtxt<'tcx>, ()) -> &'tcx ty::ResolverGlobalCtxt;

/// 本地定义范围查询的函数签名。
type SpanQuery = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Span;

/// 本地定义标识符范围查询的函数签名。
type IdentSpanQuery = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> Option<Span>;

/// 未包装的外部可见性查询，供审计恢复真实边界。
static VISIBILITY: OnceLock<VisibilityQuery> = OnceLock::new();

/// 未包装的模块子项查询，供恢复原始公共导出。
static CHILDREN: OnceLock<ChildrenQuery> = OnceLock::new();

/// 未包装的名称解析结果查询。
static RESOLUTIONS: OnceLock<ResolutionsQuery> = OnceLock::new();

/// 未包装的定义范围查询，供附加生成来源。
static DEF_SPAN: OnceLock<SpanQuery> = OnceLock::new();

/// 未包装的标识符范围查询。
static DEF_IDENT_SPAN: OnceLock<IdentSpanQuery> = OnceLock::new();

/// 当前编译的精确生成区间；每次调用都替换，不跨编译继承。
static TRUSTED_RANGES: Mutex<Vec<TrustedRange>> = Mutex::new(Vec::new());

/// CLI 选定的真实 bridge 工件路径，不以同名 crate 代替认证。
static BRIDGE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The CLI-selected bridge artifact, including for transitive-only consumers.
/// A different dependency with the same crate name is not this compiler tool.
pub fn set_bridge_path(path: PathBuf) {
    *BRIDGE_PATH.lock().expect("bridge identity lock poisoned") = Some(path);
}

/// 按实际加载工件核对 extern 身份，不能只相信相同 crate 名称。
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
/// 识别当前加载的 core 身份；名称解析期间只检查传入 crate，不冻结其他 extern。
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

/// 核对 CLI 选定的 bridge 工件，独立回归环境则要求显式 extern 匹配。
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
    /// 当前覆盖层对应的真实源文件路径。
    pub path: PathBuf,

    /// 生成片段的起始 UTF-8 字节偏移，包含该位置。
    pub start: usize,

    /// 生成片段结束的 UTF-8 字节偏移，不包含该位置。
    pub end: usize,
}

/// Replace the ranges for each compiler invocation, including empty discovery
/// passes, so permissions cannot leak from a previous compilation.
pub fn set_trusted_ranges(ranges: Vec<TrustedRange>) {
    *TRUSTED_RANGES
        .lock()
        .expect("internal source range lock poisoned") = ranges;
}

/// 安装临时名称解析与生成来源钩子；必须与类型检查后的访问审计配套使用。
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

/// 在原定义范围上附加已认证生成来源，保留原始位置。
fn definition_span(tcx: TyCtxt<'_>, definition: LocalDefId) -> Span {
    let original = (DEF_SPAN.get().expect("Nestrs span hook not installed"))(tcx, definition);
    mark_generated(tcx, original)
}

/// 仅对存在标识符位置的定义附加生成来源。
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

/// 以 core 私有计划 ABI 的完整定义路径和签名认证生成卫生锚点。
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
        && matches!(signature.inputs(), [pointer, eager, scope_eager, concurrency]
            if matches!(pointer.kind(), ty::RawPtr(element, mutability)
                if *element == tcx.types.unit && mutability.is_mut())
                && *eager == tcx.types.bool
                && *scope_eager == tcx.types.bool
                && *concurrency == tcx.types.usize)
}

/// 按定义所属 crate 检查其是否来自当前 runtime。
fn is_runtime(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    !definition.is_local() && runtime_crate(tcx, definition.krate)
}

/// 仅在解析阶段临时开放 runtime 名称，其他定义使用原始可见性。
fn visibility(tcx: TyCtxt<'_>, definition: DefId) -> ty::Visibility<ModId> {
    if is_runtime(tcx, definition) {
        ty::Visibility::Public
    } else {
        original_visibility(tcx, definition)
    }
}

/// 绕过临时包装，读取定义的原始 Rust 可见性。
fn original_visibility(tcx: TyCtxt<'_>, definition: DefId) -> ty::Visibility<ModId> {
    (VISIBILITY
        .get()
        .expect("Nestrs visibility hook not installed"))(tcx, definition)
}

/// 向生成代码的名称解析提供 runtime 子项；普通依赖保持原子项表。
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

/// 复制导出记录及重导出链，供过滤后的解析结果持有。
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

/// 核对 HIR 对 core 私有实现的访问是否来自认证生成代码。
struct Audit<'tcx> {
    /// 当前会话的定义、类型与源码查询入口。
    tcx: TyCtxt<'tcx>,

    /// 原始 core 公共导出集合，不包含临时暴露的名字。
    public: HashSet<DefId>,

    /// 本轮编译的精确生成区间快照。
    ranges: Vec<TrustedRange>,

    /// 已报告的定义与源码位置，避免重复诊断。
    errors: HashSet<(DefId, Span)>,
}

impl<'tcx> Audit<'tcx> {
    /// 缓存原始公共导出与本轮可信区间，随后按真实定义身份审计。
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

    /// 使用本次审计的范围快照检查调用位置来源。
    fn trusted(&self, span: Span) -> bool {
        trusted_span_in(self.tcx, span, &self.ranges)
    }

    /// 沿关联项和 impl 所有者判断是否越过 core 公共门面。
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

    /// 对未认证来源访问的私有定义发出一次业务位置错误。
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

    /// 仅对已解析的定义引用执行权限审计。
    fn resolution(&mut self, resolution: Res, span: Span) {
        if let Res::Def(_, definition) = resolution {
            self.check(definition, span);
        }
    }

    /// 检查推断类型中的私有 ADT、函数项和 trait 身份，覆盖未显式写出的访问。
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

/// 核对 bridge 宏卫生或当前编译器生成区间中的来源身份。
pub fn trusted_span(tcx: TyCtxt<'_>, span: Span) -> bool {
    trusted_span_in(
        tcx,
        span,
        &TRUSTED_RANGES
            .lock()
            .expect("internal source range lock poisoned"),
    )
}

/// 沿真实展开链与精确字节区间验证来源，不以文件名或标记拼写授予权限。
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
    /// 审计整个 HIR 的嵌套定义。
    type NestedFilter = rustc_middle::hir::nested_filter::All;

    /// 向完整 HIR 遍历提供当前类型上下文。
    fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
        self.tcx
    }

    /// 审计路径解析到的定义，并继续遍历泛型参数。
    fn visit_path(&mut self, path: &hir::Path<'tcx>, _: hir::HirId) {
        self.resolution(path.res, path.span);
        intravisit::walk_path(self, path);
    }

    /// 覆盖类型检查后才能确认的关联路径段。
    fn visit_path_segment(&mut self, segment: &hir::PathSegment<'tcx>) {
        self.resolution(segment.res, segment.ident.span);
        if let Some(arguments) = segment.args {
            self.visit_generic_args(arguments);
        }
    }

    /// 审计每个 use 解析目标，防止私有实现经重导出流出。
    fn visit_use(&mut self, path: &'tcx hir::UsePath<'tcx>, id: hir::HirId) {
        for resolution in [path.res.type_ns, path.res.value_ns, path.res.macro_ns]
            .into_iter()
            .flatten()
        {
            self.resolution(resolution, path.span);
        }
        intravisit::walk_use(self, path, id);
    }

    /// 同时检查表达式的显式解析与推断类型，覆盖方法和字段等间接访问。
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
