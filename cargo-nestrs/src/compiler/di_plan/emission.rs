//! 编译器计划的版本化入口及 MIR 装配。
//!
//! 这里调用真实 Rust typed sink，并在生成每个调用前核对完整函数签名。生成代码不
//! 构造枚举布局、不写入宿主指针、不绕过业务借用检查；服务的构造入口仍由标准 Rust
//! 编译得到。入口只在最终binary/test存在，rlib不导出相互竞争的全局符号。

use super::*;
use rustc_abi::ExternAbi;
use rustc_ast::{self as ast, token};
use rustc_data_structures::steal::Steal;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefIndex, LocalDefId};
use rustc_index::{Idx, IndexVec};
use rustc_interface::interface;
use rustc_middle::{
    middle::codegen_fn_attrs::CodegenFnAttrs,
    mir::{BasicBlock, BasicBlockData, Local, Operand, Place, TerminatorKind},
};
use rustc_span::{FileName, Spanned, Symbol};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

const ENTRY: &str = "__nestrs_reflect_v1";
const SOURCE: &str = "nestrs generated reflection plan";
static ENABLED: AtomicBool = AtomicBool::new(false);
type MirBuilt = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<mir::Body<'tcx>>;
type Attrs = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> CodegenFnAttrs;
static MIR: OnceLock<MirBuilt> = OnceLock::new();
static ATTRS: OnceLock<Attrs> = OnceLock::new();
pub fn enable(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}
pub fn provide(providers: &mut rustc_middle::util::Providers) {
    MIR.get_or_init(|| providers.queries.mir_built);
    ATTRS.get_or_init(|| providers.queries.codegen_fn_attrs);
    providers.queries.mir_built = plan_mir;
    providers.queries.codegen_fn_attrs = plan_attrs;
}
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
    let mut parser = rustc_parse::new_parser_from_source_str(
        &compiler.sess.psess,
        FileName::Custom(SOURCE.into()),
        format!("#[doc(hidden)] #[allow(dead_code)] fn {ENTRY}(_output: *mut ()) {{}}"),
        rustc_parse::lexer::StripTokens::Nothing,
    )
    .unwrap_or_else(|errors| {
        for error in errors {
            error.emit();
        }
        compiler.sess.dcx().fatal("无法生成 Nestrs 编译计划入口")
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
pub fn is_entry(tcx: TyCtxt<'_>, def: LocalDefId) -> bool {
    if tcx.def_kind(def) != DefKind::Fn
        || tcx.is_foreign_item(def.to_def_id())
        || tcx
            .opt_item_name(def.to_def_id())
            .is_none_or(|n| n.as_str() != ENTRY)
    {
        return false;
    }
    let file = tcx
        .sess
        .source_map()
        .lookup_source_file(tcx.def_span(def).lo());
    if !matches!(&file.name,FileName::Custom(name) if name==SOURCE) {
        tcx.dcx().fatal("Nestrs 计划入口名称由编译器保留");
    }
    true
}
fn plan_attrs(tcx: TyCtxt<'_>, def: LocalDefId) -> CodegenFnAttrs {
    let mut attrs = ATTRS.get().unwrap()(tcx, def);
    if is_entry(tcx, def) {
        attrs.symbol_name = Some(Symbol::intern(ENTRY));
    }
    attrs
}
fn plan_mir(tcx: TyCtxt<'_>, def: LocalDefId) -> &Steal<mir::Body<'_>> {
    let original = MIR.get().unwrap()(tcx, def);
    if !is_entry(tcx, def) || !ENABLED.load(Ordering::Relaxed) || !has_core(tcx) {
        return original;
    }
    let plan = compile(tcx).unwrap_or_else(|error| tcx.dcx().fatal(error));
    let mut body = original.steal();
    let info = body.basic_blocks[mir::START_BLOCK].terminator().source_info;
    let helpers = helpers(tcx);
    let mut writer = Writer {
        tcx,
        body: &mut body,
        blocks: IndexVec::new(),
        info,
        helpers,
    };
    let config = crate::registration_codegen::startup_options(tcx.sess);
    writer.sink(
        "plan_set_options",
        vec![
            writer.pointer(),
            boolean(tcx, config.eager, info.span),
            number(tcx, config.max_concurrent_activations, info.span),
        ],
    );
    for binding in &plan.bindings {
        let value = writer.descriptor(binding.instance, "activation::adapter::ProjectionAdapter");
        writer.sink("plan_push_binding", vec![writer.pointer(), value]);
    }
    for (id, provider) in plan.providers.iter().enumerate() {
        // 泛型蓝图的普通 helper 属于声明所在匿名作用域；调用它保留真实 trait
        // 约束检查，且不把私有类型变成下游需要命名的 Rust 源码路径。
        let callback = if tcx
            .trait_impl_of_assoc(provider.instance.def_id())
            .is_some()
        {
            crate::autobind_semantic::provider_callback(tcx, plan.types[provider.data.type_id])
                .unwrap_or_else(|error| tcx.dcx().fatal(error))
        } else {
            provider.instance
        };
        let value = writer.descriptor(callback, "activation::adapter::ActivationAdapter");
        let loc = tcx
            .sess
            .source_map()
            .lookup_char_pos(provider.source.source_callsite().lo());
        let source_file = loc.file.name.prefer_local_unconditionally().to_string();
        let (key_kind, key_name, key_index) = key_parts(&provider.data.key);
        let lifetime = match provider.data.lifetime {
            model::Lifetime::Singleton => 0,
            model::Lifetime::Scoped => 1,
            model::Lifetime::Transient => 2,
        };
        let initialization = match provider.data.lazy {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        };
        writer.sink(
            "plan_push_provider",
            vec![
                writer.pointer(),
                value,
                number(tcx, lifetime, info.span),
                number(tcx, key_kind, info.span),
                text(tcx, key_name, info.span),
                number(tcx, key_index, info.span),
                number(tcx, initialization, info.span),
                text(tcx, &source_file, info.span),
                number(tcx, loc.line, info.span),
                number(tcx, loc.col.0 + 1, info.span),
                boolean(tcx, plan.plan.requires_scope[id], info.span),
            ],
        );
    }
    for (id, inputs) in plan.plan.inputs.iter().enumerate() {
        for (slot, input) in inputs.iter().enumerate() {
            let declaration = &plan.providers[id].data.inputs[slot];
            let (key_kind, key_name, key_index) = key_parts(&declaration.key);
            writer.sink(
                "plan_set_input",
                vec![
                    writer.pointer(),
                    number(tcx, id, info.span),
                    number(tcx, slot, info.span),
                    optional_number(tcx, input.target, info.span),
                    optional_number(tcx, input.binding, info.span),
                    boolean(tcx, declaration.optional, info.span),
                    number(tcx, key_kind, info.span),
                    text(tcx, key_name, info.span),
                    number(tcx, key_index, info.span),
                    text(tcx, &declaration.label, info.span),
                ],
            );
        }
    }
    for route in &plan.plan.routes {
        if let Some(binding) = route.binding {
            writer.sink(
                "plan_push_trait_route",
                vec![
                    writer.pointer(),
                    number(tcx, route.provider, info.span),
                    number(tcx, binding, info.span),
                ],
            );
        }
    }
    for &id in &plan.plan.order {
        writer.sink(
            "plan_push_order",
            vec![writer.pointer(), number(tcx, id, info.span)],
        );
    }
    for (dependency, consumers) in plan.plan.dependents.iter().enumerate() {
        for &consumer in consumers {
            writer.sink(
                "plan_push_dependent",
                vec![
                    writer.pointer(),
                    number(tcx, dependency, info.span),
                    number(tcx, consumer, info.span),
                ],
            );
        }
    }
    writer.blocks.push(BasicBlockData::new(
        Some(mir::Terminator {
            source_info: info,
            kind: TerminatorKind::Return,
            attributes: Default::default(),
        }),
        false,
    ));
    let blocks = writer.blocks;
    body.basic_blocks = mir::BasicBlocks::new(blocks);
    tcx.alloc_steal_mir(body)
}
fn helpers(tcx: TyCtxt<'_>) -> HashMap<String, DefId> {
    let mut result = HashMap::new();
    let mut crates = tcx.crates(()).to_vec();
    if tcx.crate_name(LOCAL_CRATE).as_str() == "nestrs_core" {
        crates.push(LOCAL_CRATE);
    }
    for c in crates
        .into_iter()
        .filter(|&c| tcx.crate_name(c).as_str() == "nestrs_core")
    {
        let definitions: Vec<_> = if c == LOCAL_CRATE {
            tcx.hir_body_owners().map(LocalDefId::to_def_id).collect()
        } else {
            (0..tcx.num_extern_def_ids(c))
                .map(|i| DefId {
                    krate: c,
                    index: DefIndex::from_usize(i),
                })
                .filter(|&d| tcx.is_mir_available(d))
                .collect()
        };
        for def in definitions {
            if tcx.def_kind(def) == DefKind::Fn
                && let Some(name) = tcx.opt_item_name(def)
                && name.as_str().starts_with("plan_")
                && definition_path(tcx, def) == format!("graph::plan::{name}")
            {
                result.insert(name.to_string(), def);
            }
        }
    }
    result
}
struct Writer<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'a mut mir::Body<'tcx>,
    blocks: IndexVec<BasicBlock, BasicBlockData<'tcx>>,
    info: mir::SourceInfo,
    helpers: HashMap<String, DefId>,
}
impl<'tcx> Writer<'_, 'tcx> {
    fn pointer(&self) -> Operand<'tcx> {
        Operand::Copy(Local::new(1).into())
    }
    fn call(
        &mut self,
        instance: ty::Instance<'tcx>,
        args: Vec<Operand<'tcx>>,
        output: Ty<'tcx>,
    ) -> Operand<'tcx> {
        let local = self
            .body
            .local_decls
            .push(mir::LocalDecl::new(output, self.info.span));
        let target = BasicBlock::new(self.blocks.len() + 1);
        self.blocks.push(BasicBlockData::new(
            Some(mir::Terminator {
                source_info: self.info,
                kind: TerminatorKind::Call {
                    func: constant(
                        mir::Const::zero_sized(Ty::new_fn_def(
                            self.tcx,
                            instance.def_id(),
                            instance.args,
                        )),
                        self.info.span,
                    ),
                    args: args
                        .into_iter()
                        .map(|node| Spanned {
                            node,
                            span: self.info.span,
                        })
                        .collect(),
                    destination: Place::from(local),
                    target: Some(target),
                    unwind: mir::UnwindAction::Continue,
                    call_source: mir::CallSource::Misc,
                    fn_span: self.info.span,
                },
                attributes: Default::default(),
            }),
            false,
        ));
        Operand::Move(local.into())
    }
    fn descriptor(&mut self, instance: ty::Instance<'tcx>, expected: &str) -> Operand<'tcx> {
        let sig = self
            .tcx
            .fn_sig(instance.def_id())
            .instantiate(self.tcx, instance.args)
            .skip_normalization()
            .skip_binder();
        if !sig.inputs().is_empty()
            || !sig.safety().is_safe()
            || sig.abi() != ExternAbi::Rust
            || sig.c_variadic()
            || !matches!(sig.output().kind(),ty::Adt(d,_) if self.tcx.crate_name(d.did().krate).as_str()=="nestrs_core" && definition_path(self.tcx,d.did())==expected)
        {
            self.tcx.dcx().fatal("不兼容的 Nestrs typed descriptor ABI");
        }
        self.call(instance, vec![], sig.output())
    }
    fn sink(&mut self, name: &str, args: Vec<Operand<'tcx>>) {
        let def = *self.helpers.get(name).unwrap_or_else(|| {
            self.tcx
                .dcx()
                .fatal(format!("缺少 Nestrs plan ABI {name}；请重建工具链与 core"))
        });
        let sig = self
            .tcx
            .fn_sig(def)
            .instantiate_identity()
            .skip_normalization()
            .skip_binder();
        let input_types: Vec<_> = args
            .iter()
            .map(|v| v.ty(&self.body.local_decls, self.tcx))
            .collect();
        if self.tcx.generics_of(def).count() != 0
            || sig.safety().is_safe()
            || sig.abi() != ExternAbi::Rust
            || sig.c_variadic()
            || sig.inputs() != input_types
            || sig.output() != self.tcx.types.unit
        {
            self.tcx
                .dcx()
                .fatal(format!("不兼容的 Nestrs plan sink ABI: {name}"));
        }
        self.call(ty::Instance::mono(self.tcx, def), args, self.tcx.types.unit);
    }
}
fn constant(value: mir::Const<'_>, span: Span) -> Operand<'_> {
    Operand::Constant(Box::new(mir::ConstOperand {
        span,
        user_ty: None,
        const_: value,
    }))
}
fn number(tcx: TyCtxt<'_>, value: usize, span: Span) -> Operand<'_> {
    constant(mir::Const::from_usize(tcx, value as u64), span)
}
fn optional_number(tcx: TyCtxt<'_>, value: Option<usize>, span: Span) -> Operand<'_> {
    // None使用目标usize::MAX，不能把64位宿主哨兵截断当作类型协议。
    let value = value
        .map(|v| v as u64)
        .unwrap_or_else(|| u64::MAX >> (64 - tcx.sess.target.pointer_width));
    constant(mir::Const::from_usize(tcx, value), span)
}
fn boolean(tcx: TyCtxt<'_>, value: bool, span: Span) -> Operand<'_> {
    constant(mir::Const::from_bool(tcx, value), span)
}

/// 字符串是普通目标端 Rust 常量，由 rustc 分配常量内存；绝不序列化宿主指针。
fn text<'tcx>(tcx: TyCtxt<'tcx>, value: &str, span: Span) -> Operand<'tcx> {
    let alloc_id = tcx.allocate_bytes_dedup(
        value.as_bytes(),
        rustc_middle::mir::interpret::CTFE_ALLOC_SALT,
    );
    let value = mir::ConstValue::Slice {
        alloc_id,
        meta: value.len() as u64,
    };
    constant(
        mir::Const::Val(
            value,
            Ty::new_imm_ref(tcx, tcx.lifetimes.re_static, tcx.types.str_),
        ),
        span,
    )
}

fn key_parts(key: &model::Key) -> (usize, &str, usize) {
    match key {
        model::Key::Default => (0, "", 0),
        model::Key::Named(value) => (1, value, 0),
        model::Key::Indexed(value) => (
            2,
            "",
            usize::try_from(*value).expect("compiler key fits target usize"),
        ),
    }
}
