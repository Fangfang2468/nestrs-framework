//! Compiler-owned aggregation of typed DI declarations.
//!
//! Each executable (including a test harness) defines one private registry
//! entry. Its body calls the actual declaration DefIds, so anonymous consts,
//! function-local query roots and private upstream declarations need no Rust
//! re-exports. The entry replaces distributed linker sections with an explicit
//! call sequence. It only evaluates descriptors; constructors remain callbacks.

extern crate rustc_abi;
extern crate rustc_data_structures;
extern crate rustc_index;

use rustc_abi::ExternAbi;
use rustc_ast::{self as ast, token};
use rustc_data_structures::steal::Steal;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, DefIndex, LOCAL_CRATE, LocalDefId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_index::{Idx, IndexVec};
use rustc_interface::interface;
use rustc_middle::middle::codegen_fn_attrs::CodegenFnAttrs;
use rustc_middle::mir::{self, BasicBlock, BasicBlockData, Local, Operand, Place, TerminatorKind};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::{FileName, Span, Spanned, Symbol};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::OnceLock;

/// This symbol is owned by the compiler/runtime ABI, never by application API.
pub const ENTRY_NAME: &str = "__nestrs_registry_v1";
const ENTRY_SOURCE: &str = "nestrs compiler registry";
const OPTIONS_HELPER: &str = "registry_set_options";

type MirBuilt = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<mir::Body<'tcx>>;
static ORIGINAL_MIR_BUILT: OnceLock<MirBuilt> = OnceLock::new();
type CodegenAttrs = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> CodegenFnAttrs;
static ORIGINAL_CODEGEN_ATTRS: OnceLock<CodegenAttrs> = OnceLock::new();

/// Install alongside the other compiler overrides, in the same order in both
/// compiler passes. Calling the original provider preserves normal MIR setup.
pub fn provide(providers: &mut rustc_middle::util::Providers) {
    ORIGINAL_MIR_BUILT.get_or_init(|| providers.queries.mir_built);
    providers.queries.mir_built = registry_mir;
    ORIGINAL_CODEGEN_ATTRS.get_or_init(|| providers.queries.codegen_fn_attrs);
    providers.queries.codegen_fn_attrs = registry_codegen_attrs;
}

/// Add only a signature before normal name resolution/type checking. The
/// executable owns the symbol; rlibs never contribute competing definitions.
pub fn prepare(compiler: &interface::Compiler, krate: &mut ast::Crate) {
    if !compiler.sess.opts.test
        && !compiler
            .sess
            .opts
            .crate_types
            .contains(&rustc_session::config::CrateType::Executable)
    {
        return;
    }
    // 在两个编译阶段都读取并校验当前入口的 package 配置。即使 cargo check 不生成
    // 最终目标代码，配置错误也会当场报告；不能等到首次构图才发现拼写或数值错误。
    startup_options(&compiler.sess);
    // Exporting the symbol is compiler work, not an unsafe source declaration
    // in the user's crate. This also preserves #![forbid(unsafe_code)].
    let source =
        format!("#[doc(hidden)] #[allow(dead_code)] fn {ENTRY_NAME}(_output: *mut ()) {{}}");
    let mut parser = rustc_parse::new_parser_from_source_str(
        &compiler.sess.psess,
        FileName::Custom(ENTRY_SOURCE.into()),
        source,
        rustc_parse::lexer::StripTokens::Nothing,
    )
    .unwrap_or_else(|diagnostics| {
        for diagnostic in diagnostics {
            diagnostic.emit();
        }
        compiler
            .sess
            .dcx()
            .fatal("cannot parse the Nestrs registry entry")
    });
    while parser.token != token::Eof {
        match parser.parse_item(
            rustc_parse::parser::ForceCollect::No,
            rustc_parse::parser::AllowConstBlockItems::No,
        ) {
            Ok(Some(item)) => krate.items.push(item),
            Ok(None) => break,
            Err(error) => {
                error.emit();
                break;
            }
        }
    }
}

/// 配置归最终入口所属 package 所有。CARGO_MANIFEST_* 由 Cargo 为每个编译单元设置，
/// 因此上游 rlib 的 metadata 不会把自己的启动策略传播到宿主 binary/test。
///
/// 使用 SourceMap 而非直接 fs::read，令 Cargo.toml 同时进入 rustc dep-info 和现有
/// SnapshotLoader/CheckedLoader：仅修改 [nestrs-cli] 也必须重编译，两个编译阶段之间
/// 改动文件则拒绝继续。编译后的程序只含配置值，运行时无需部署或读取 Cargo.toml。
fn startup_options(session: &rustc_session::Session) -> cargo_nestrs::project_config::DiConfig {
    let manifest = std::env::var_os("CARGO_MANIFEST_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("CARGO_MANIFEST_DIR")
                .map(|directory| PathBuf::from(directory).join("Cargo.toml"))
        });
    let Some(manifest) = manifest else {
        // 直接调用 rustc_driver 的 ABI 回归探针不是 Cargo 项目，保留原有缺省行为。
        return cargo_nestrs::project_config::DiConfig::default();
    };
    let source = session
        .source_map()
        .load_file(&manifest)
        .unwrap_or_else(|error| {
            session.dcx().fatal(format!(
                "无法读取入口项目配置 {}：{error}",
                manifest.display(),
            ))
        });
    let source = source.src.as_deref().unwrap_or_else(|| {
        session.dcx().fatal(format!(
            "入口项目配置没有可读取的内容：{}",
            manifest.display(),
        ))
    });
    let options = cargo_nestrs::project_config::parse_manifest(source).unwrap_or_else(|error| {
        session
            .dcx()
            .fatal(format!("{}：{error}", manifest.display()))
    });
    // Driver 自身可能是 64 位，而应用是 32 位目标。不得将宿主 usize 配置静默截断。
    let target_max = u128::MAX >> (128 - session.target.pointer_width);
    if options.max_concurrent_activations as u128 > target_max {
        session.dcx().fatal(format!(
            "{}：[nestrs-cli].max-concurrent-activations 超出 {} 位目标 usize 的范围",
            manifest.display(),
            session.target.pointer_width,
        ));
    }
    options
}

/// The generated source signature is intentionally safe so it does not add an
/// unsafe item to a #![forbid(unsafe_code)] application. It is not a callable
/// Rust API: the runtime's foreign ABI call is the sole permitted entry. Audit
/// every source expression, including function-item values and never-executed
/// bodies, before any code generation can make an invalid pointer reachable.
pub fn validate(tcx: TyCtxt<'_>) {
    struct References<'tcx> {
        tcx: TyCtxt<'tcx>,
    }
    impl References<'_> {
        fn check(&self, definition: DefId, span: Span) {
            if let Some(local) = definition.as_local()
                && is_entry(self.tcx, local)
            {
                self.tcx.dcx().span_fatal(
                    span,
                    "the compiler-owned Nestrs registry entry cannot be referenced by Rust source",
                );
            }
        }
    }
    impl<'tcx> Visitor<'tcx> for References<'tcx> {
        type NestedFilter = rustc_middle::hir::nested_filter::All;
        fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
            self.tcx
        }
        fn visit_path(&mut self, path: &rustc_hir::Path<'tcx>, _: rustc_hir::HirId) {
            if let Some(definition) = path.res.opt_def_id() {
                self.check(definition, path.span);
            }
            intravisit::walk_path(self, path);
        }
        fn visit_use(&mut self, path: &'tcx rustc_hir::UsePath<'tcx>, id: rustc_hir::HirId) {
            for resolution in [path.res.type_ns, path.res.value_ns, path.res.macro_ns]
                .into_iter()
                .flatten()
            {
                if let Some(definition) = resolution.opt_def_id() {
                    self.check(definition, path.span);
                }
            }
            intravisit::walk_use(self, path, id);
        }
        fn visit_expr(&mut self, expression: &'tcx rustc_hir::Expr<'tcx>) {
            let owner = expression.hir_id.owner.def_id;
            if self.tcx.has_typeck_results(owner)
                && let Some(value) = self.tcx.typeck(owner).node_type_opt(expression.hir_id)
                && let ty::FnDef(definition, _) = *value.kind()
            {
                self.check(definition, expression.span);
            }
            intravisit::walk_expr(self, expression);
        }
    }
    tcx.hir_walk_toplevel_module(&mut References { tcx });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Provider,
    Binding,
    Root,
    AutomaticBinding,
    Blueprint,
}

impl Kind {
    fn from_callback(name: &str) -> Option<Self> {
        match name {
            "__nestrs_reflect_provider" | "__nestrs_reflected_factory" => Some(Self::Provider),
            "__nestrs_reflect_trait_binding" => Some(Self::Binding),
            "__nestrs_query_root" => Some(Self::Root),
            "__nestrs_reflect_automatic_binding" => Some(Self::AutomaticBinding),
            "__nestrs_reflect_blueprint" | "__nestrs_reflect_blueprint_path" => {
                Some(Self::Blueprint)
            }
            _ => None,
        }
    }

    fn helper(self) -> &'static str {
        match self {
            Self::Provider => "registry_push_provider",
            Self::Binding => "registry_push_binding",
            Self::Root => "registry_push_root",
            Self::AutomaticBinding => "registry_push_automatic_binding",
            Self::Blueprint => "registry_push_blueprint",
        }
    }

    fn descriptor(self) -> &'static str {
        match self {
            Self::Provider => "registration::provider::Provider",
            Self::Binding | Self::AutomaticBinding => "registration::binding::TraitBinding",
            Self::Root | Self::Blueprint => "registration::root::RootDeclaration",
        }
    }
}

fn definition_path(tcx: TyCtxt<'_>, definition: DefId) -> String {
    tcx.def_path(definition)
        .data
        .iter()
        .map(|component| {
            component
                .data
                .get_opt_name()
                .map_or_else(|| "<anonymous>".into(), |name| name.to_string())
        })
        .collect::<Vec<_>>()
        .join("::")
}

fn registry_mir(tcx: TyCtxt<'_>, definition: LocalDefId) -> &Steal<mir::Body<'_>> {
    let original = ORIGINAL_MIR_BUILT
        .get()
        .expect("Nestrs registry MIR provider was installed")(tcx, definition);
    if !is_entry(tcx, definition) {
        return original;
    }
    fill_registry_mir(tcx, original)
}

fn is_entry(tcx: TyCtxt<'_>, definition: LocalDefId) -> bool {
    // The core contains the corresponding foreign declaration. Only a local
    // function definition can be the generated implementation.
    if tcx.def_kind(definition) != DefKind::Fn || tcx.is_foreign_item(definition.to_def_id()) {
        return false;
    }
    if tcx
        .opt_item_name(definition.to_def_id())
        .is_none_or(|name| name.as_str() != ENTRY_NAME)
    {
        return false;
    }
    let source = tcx
        .sess
        .source_map()
        .lookup_source_file(tcx.def_span(definition).lo());
    if !matches!(&source.name, FileName::Custom(name) if name == ENTRY_SOURCE) {
        tcx.dcx()
            .fatal("the Nestrs registry entry name is reserved for the compiler");
    }
    true
}

fn registry_codegen_attrs(tcx: TyCtxt<'_>, definition: LocalDefId) -> CodegenFnAttrs {
    let original = ORIGINAL_CODEGEN_ATTRS
        .get()
        .expect("registry symbol provider was installed")(tcx, definition);
    if !is_entry(tcx, definition) {
        return original;
    }
    let mut attrs = original;
    attrs.symbol_name = Some(Symbol::intern(ENTRY_NAME));
    attrs
}

fn fill_registry_mir<'tcx>(
    tcx: TyCtxt<'tcx>,
    original: &'tcx Steal<mir::Body<'tcx>>,
) -> &'tcx Steal<mir::Body<'tcx>> {
    let callbacks = collect_callbacks(tcx);
    let (helpers, options_helper) = collect_helpers(tcx);
    let mut body = original.steal();
    let source_info = body.basic_blocks[mir::START_BLOCK].terminator().source_info;
    let mut blocks = IndexVec::new();
    if let Some(helper) = options_helper {
        validate_options_helper(tcx, helper, body.local_decls[Local::new(1)].ty);
        let options = startup_options(tcx.sess);
        let unit = body
            .local_decls
            .push(mir::LocalDecl::new(tcx.types.unit, source_info.span));
        // 只有最终入口写一次配置，随后才汇总本 crate 和依赖中的服务描述。
        // 传入普通标量并调用真实 typed sink，不伪造 options 内存布局或 enum 判别值。
        blocks.push(call_block(
            tcx,
            helper,
            vec![
                Operand::Copy(Local::new(1).into()),
                constant_operand(mir::Const::from_bool(tcx, options.eager), source_info.span),
                constant_operand(
                    mir::Const::from_usize(tcx, options.max_concurrent_activations as u64),
                    source_info.span,
                ),
            ],
            unit.into(),
            BasicBlock::new(1),
            source_info,
        ));
    }
    for (kind, callback) in callbacks {
        let helper = *helpers.get(&kind).unwrap_or_else(|| {
            tcx.dcx().fatal(format!(
                "Nestrs registry ABI is missing {}; rebuild nestrs-core with compatible cargo nestrs",
                kind.helper(),
            ))
        });
        let output = validate_callback(
            tcx,
            kind,
            callback,
            helper,
            body.local_decls[Local::new(1)].ty,
        );
        let value = body
            .local_decls
            .push(mir::LocalDecl::new(output, tcx.def_span(callback)));
        let unit = body
            .local_decls
            .push(mir::LocalDecl::new(tcx.types.unit, source_info.span));
        let next = BasicBlock::new(blocks.len() + 1);
        blocks.push(call_block(
            tcx,
            callback,
            vec![],
            value.into(),
            next,
            source_info,
        ));
        let next = BasicBlock::new(blocks.len() + 1);
        blocks.push(call_block(
            tcx,
            helper,
            vec![
                Operand::Copy(Local::new(1).into()),
                Operand::Move(value.into()),
            ],
            unit.into(),
            next,
            source_info,
        ));
    }
    blocks.push(BasicBlockData::new(
        Some(mir::Terminator {
            source_info,
            kind: TerminatorKind::Return,
            attributes: Default::default(),
        }),
        false,
    ));
    body.basic_blocks = mir::BasicBlocks::new(blocks);
    tcx.alloc_steal_mir(body)
}

fn collect_callbacks(tcx: TyCtxt<'_>) -> Vec<(Kind, DefId)> {
    let mut definitions: Vec<_> = tcx
        .hir_body_owners()
        .filter(|definition| tcx.def_kind(*definition) == DefKind::Fn)
        .map(LocalDefId::to_def_id)
        .collect();
    for &krate in tcx.crates(()) {
        for index in 0..tcx.num_extern_def_ids(krate) {
            let definition = DefId {
                krate,
                index: DefIndex::from_usize(index),
            };
            // Upstream metadata tables have holes. MIR availability is safe to
            // query first, and also proves the callback can be instantiated.
            if tcx.is_mir_available(definition) {
                definitions.push(definition);
            }
        }
    }
    let mut callbacks: Vec<_> = definitions
        .into_iter()
        .filter_map(|definition| callback_kind(tcx, definition).map(|kind| (kind, definition)))
        .collect();
    callbacks.sort_by_cached_key(|(kind, definition)| {
        (
            *kind,
            tcx.def_path_str(*definition),
            format!("{:?}", tcx.def_path_hash(*definition)),
        )
    });
    callbacks.dedup();
    callbacks
}

/// The producer-retention pass and final executable collector authenticate the
/// same callbacks. A reserved spelling alone never grants compiler authority.
pub(crate) fn authenticated_callback(tcx: TyCtxt<'_>, definition: DefId) -> bool {
    let Some(kind) = callback_kind(tcx, definition) else {
        return false;
    };
    callback_descriptor(tcx, kind, definition);
    true
}

fn callback_kind(tcx: TyCtxt<'_>, definition: DefId) -> Option<Kind> {
    let kind = Kind::from_callback(tcx.opt_item_name(definition)?.as_str())?;
    if tcx.def_kind(definition) != DefKind::Fn
        || tcx.is_foreign_item(definition)
        || !crate::internal_access::trusted_definition(tcx, definition)
    {
        tcx.dcx().span_fatal(
            tcx.def_span(definition),
            "Nestrs registration callback names are reserved for authenticated declarations",
        );
    }
    Some(kind)
}

fn callback_descriptor<'tcx>(tcx: TyCtxt<'tcx>, kind: Kind, callback: DefId) -> Ty<'tcx> {
    if tcx.generics_of(callback).count() != 0 {
        tcx.dcx()
            .fatal("Nestrs registry callbacks must have closed signatures");
    }
    let declaration = tcx
        .fn_sig(callback)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    let output = declaration.output();
    let expected_descriptor = matches!(output.kind(), ty::Adt(definition, _) if
        tcx.crate_name(definition.did().krate).as_str() == "nestrs_core"
        && definition_path(tcx, definition.did()) == kind.descriptor());
    if declaration.abi() != ExternAbi::Rust
        || !declaration.safety().is_safe()
        || declaration.c_variadic()
        || !declaration.inputs().is_empty()
        || !expected_descriptor
    {
        tcx.dcx().fatal(format!(
            "incompatible Nestrs registry callback ABI for {}",
            tcx.def_path_str(callback),
        ));
    }
    output
}

fn collect_helpers(tcx: TyCtxt<'_>) -> (BTreeMap<Kind, DefId>, Option<DefId>) {
    let mut cores: Vec<_> = tcx
        .crates(())
        .iter()
        .copied()
        .filter(|krate| tcx.crate_name(*krate).as_str() == "nestrs_core")
        .collect();
    if tcx.crate_name(LOCAL_CRATE).as_str() == "nestrs_core" {
        cores.push(LOCAL_CRATE);
    }
    if cores.len() > 1 {
        tcx.dcx()
            .fatal("Nestrs registry requires one compatible nestrs-core crate identity");
    }
    let Some(core) = cores.first().copied() else {
        return (BTreeMap::new(), None);
    };
    let definitions: Vec<_> = if core == LOCAL_CRATE {
        // MIR construction happens during analysis. iter_local_def_id waits
        // for completed analysis and would create a query cycle when core is
        // itself the test executable; the sinks are all ordinary HIR bodies.
        tcx.hir_body_owners()
            .filter(|definition| tcx.def_kind(*definition) == DefKind::Fn)
            .map(LocalDefId::to_def_id)
            .collect()
    } else {
        (0..tcx.num_extern_def_ids(core))
            .map(|index| DefId {
                krate: core,
                index: DefIndex::from_usize(index),
            })
            .filter(|definition| tcx.is_mir_available(*definition))
            .collect()
    };
    let mut helpers = BTreeMap::new();
    let mut options_helper = None;
    for definition in definitions {
        let Some(name) = tcx.opt_item_name(definition) else {
            continue;
        };
        if name.as_str() == OPTIONS_HELPER
            && definition_path(tcx, definition)
                == format!("registration::catalog::{OPTIONS_HELPER}")
        {
            options_helper = Some(definition);
        }
        for kind in [
            Kind::Provider,
            Kind::Binding,
            Kind::Root,
            Kind::AutomaticBinding,
            Kind::Blueprint,
        ] {
            if name.as_str() == kind.helper()
                && definition_path(tcx, definition)
                    == format!("registration::catalog::{}", kind.helper())
            {
                helpers.insert(kind, definition);
            }
        }
    }
    let options_helper = options_helper.unwrap_or_else(|| {
        tcx.dcx().fatal(format!(
            "Nestrs registry ABI is missing {OPTIONS_HELPER}; rebuild nestrs-core with compatible cargo nestrs",
        ))
    });
    (helpers, Some(options_helper))
}

/// 与描述 sink 一样核对真实 DefId 的完整 ABI，不能仅凭函数名生成任意 MIR 调用。
fn validate_options_helper<'tcx>(tcx: TyCtxt<'tcx>, helper: DefId, pointer: Ty<'tcx>) {
    if tcx.def_kind(helper) != DefKind::Fn || tcx.is_foreign_item(helper) {
        tcx.dcx()
            .fatal("Nestrs startup options sink must be a Rust function definition");
    }
    if tcx.generics_of(helper).count() != 0 {
        tcx.dcx()
            .fatal("Nestrs startup options sink must have a closed signature");
    }
    let sink = tcx
        .fn_sig(helper)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    if sink.abi() != ExternAbi::Rust
        || sink.c_variadic()
        || sink.safety().is_safe()
        || sink.inputs() != [pointer, tcx.types.bool, tcx.types.usize]
        || sink.output() != tcx.types.unit
    {
        tcx.dcx().fatal(format!(
            "incompatible Nestrs startup options ABI for {}",
            tcx.def_path_str(helper),
        ));
    }
}

fn validate_callback<'tcx>(
    tcx: TyCtxt<'tcx>,
    kind: Kind,
    callback: DefId,
    helper: DefId,
    pointer: Ty<'tcx>,
) -> Ty<'tcx> {
    let output = callback_descriptor(tcx, kind, callback);
    if tcx.generics_of(helper).count() != 0 {
        tcx.dcx()
            .fatal("Nestrs registry sinks must have closed signatures");
    }
    let sink = tcx
        .fn_sig(helper)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    if sink.abi() != ExternAbi::Rust
        || sink.c_variadic()
        || sink.safety().is_safe()
        || sink.inputs() != [pointer, output]
        || sink.output() != tcx.types.unit
    {
        tcx.dcx().fatal(format!(
            "incompatible Nestrs registry ABI for {} and {}",
            tcx.def_path_str(callback),
            tcx.def_path_str(helper),
        ));
    }
    output
}

fn function_operand<'tcx>(tcx: TyCtxt<'tcx>, definition: DefId, span: Span) -> Operand<'tcx> {
    Operand::Constant(Box::new(mir::ConstOperand {
        span,
        user_ty: None,
        const_: mir::Const::zero_sized(Ty::new_fn_def(
            tcx,
            definition,
            std::iter::empty::<ty::GenericArg<'tcx>>(),
        )),
    }))
}

fn constant_operand(value: mir::Const<'_>, span: Span) -> Operand<'_> {
    Operand::Constant(Box::new(mir::ConstOperand {
        span,
        user_ty: None,
        const_: value,
    }))
}

fn call_block<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: Vec<Operand<'tcx>>,
    destination: Place<'tcx>,
    target: BasicBlock,
    source_info: mir::SourceInfo,
) -> BasicBlockData<'tcx> {
    BasicBlockData::new(
        Some(mir::Terminator {
            source_info,
            kind: TerminatorKind::Call {
                func: function_operand(tcx, definition, source_info.span),
                args: arguments
                    .into_iter()
                    .map(|node| Spanned {
                        node,
                        span: source_info.span,
                    })
                    .collect(),
                destination,
                target: Some(target),
                // No partially owned descriptor crosses a call: a callback
                // either returns one, or its own unwind path drops it. The
                // following sink owns that descriptor by value immediately.
                unwind: mir::UnwindAction::Continue,
                call_source: mir::CallSource::Misc,
                fn_span: source_info.span,
            },
            attributes: Default::default(),
        }),
        false,
    )
}
