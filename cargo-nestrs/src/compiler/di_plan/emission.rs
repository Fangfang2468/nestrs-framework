//! 编译器计划的版本化入口及 MIR 装配。
//!
//! 这里调用真实 Rust typed sink，并在生成每个调用前核对完整函数签名。生成代码不
//! 构造枚举布局、不写入宿主指针、不绕过业务借用检查；服务的构造入口仍由标准 Rust
//! 编译得到。入口只在最终binary/test存在，rlib不导出相互竞争的全局符号。

use super::*;
use crate::protocol::{self, Initialization, KeyKind, PlanSink};
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

/// 与 core 同步版本化的唯一计划入口符号。
const ENTRY: &str = protocol::PLAN_ENTRY;

/// 生成入口使用的虚拟源码身份。
const SOURCE: &str = "nestrs generated reflection plan";

/// 当前编译阶段是否允许生成最终计划 MIR。
static ENABLED: AtomicBool = AtomicBool::new(false);

/// 原生 MIR 构建查询签名。
type MirBuilt = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<mir::Body<'tcx>>;

/// 原生代码生成属性查询签名。
type Attrs = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> CodegenFnAttrs;

/// 保存原 MIR 查询，普通函数沿用此入口。
static MIR: OnceLock<MirBuilt> = OnceLock::new();

/// 保存原代码生成属性查询，仅覆盖真实计划入口符号。
static ATTRS: OnceLock<Attrs> = OnceLock::new();

/// 控制本轮是否将已验证计划写入入口 MIR。
pub fn enable(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
}

/// 保存并包装 MIR 与代码生成属性查询，其余函数继续使用原查询。
pub fn provide(providers: &mut rustc_middle::util::Providers) {
    MIR.get_or_init(|| providers.queries.mir_built);
    ATTRS.get_or_init(|| providers.queries.codegen_fn_attrs);
    providers.queries.mir_built = plan_mir;
    providers.queries.codegen_fn_attrs = plan_attrs;
}

/// 仅为最终 binary/test 追加保留入口的普通 AST 声明，不执行目标程序。
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

/// 核对保留名称与虚拟源码来源，拒绝业务代码冒用入口。
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
    if !matches!(&file.name, FileName::Custom(name) if name == SOURCE) {
        tcx.dcx().fatal("Nestrs 计划入口名称由编译器保留");
    }
    true
}

/// 仅为真实生成入口指定固定 ABI 符号，其余代码生成属性保持原值。
fn plan_attrs(tcx: TyCtxt<'_>, def: LocalDefId) -> CodegenFnAttrs {
    let mut attrs = ATTRS.get().unwrap()(tcx, def);
    if is_entry(tcx, def) {
        attrs.symbol_name = Some(Symbol::intern(ENTRY));
    }
    attrs
}

/// 在启用且有 core 的最终入口上生成计划 MIR，普通函数返回原 body。
fn plan_mir(tcx: TyCtxt<'_>, def: LocalDefId) -> &Steal<mir::Body<'_>> {
    let original = MIR.get().unwrap()(tcx, def);
    if !is_entry(tcx, def) || !ENABLED.load(Ordering::Relaxed) || !has_core(tcx) {
        return original;
    }
    let plan = compile(tcx).unwrap_or_else(|error| crate::diagnostics::internal(tcx, error));
    let body = emit(tcx, original.steal(), &plan);
    tcx.alloc_steal_mir(body)
}

/// 只消费已检查计划并组装 MIR。mir_built 可在 analysis 的 borrowck 中提前执行；
/// 此处不发布 sidecar，产物必须等 after_analysis 的来源与两阶段一致性审计通过。
fn emit<'tcx>(
    tcx: TyCtxt<'tcx>,
    mut body: mir::Body<'tcx>,
    plan: &Compiled<'tcx>,
) -> mir::Body<'tcx> {
    let info = body.basic_blocks[mir::START_BLOCK].terminator().source_info;
    let helpers = helpers(tcx);
    let mut writer = Writer {
        tcx,
        body: &mut body,
        blocks: IndexVec::new(),
        info,
        helpers,
    };
    writer.options(&crate::registration_codegen::startup_options(tcx.sess));
    for binding in &plan.bindings {
        writer.binding(binding.instance);
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
        writer.provider(ProviderEmission {
            callback,
            declaration: provider,
            requires_scope: plan.plan.requires_scope[id],
        });
    }
    for (id, inputs) in plan.plan.inputs.iter().enumerate() {
        for (slot, input) in inputs.iter().enumerate() {
            writer.input(InputEmission {
                provider: id,
                slot,
                selected: input,
                declaration: &plan.providers[id].data.inputs[slot],
            });
        }
    }
    for route in &plan.plan.routes {
        if let Some(binding) = route.binding {
            writer.trait_route(route.provider, binding);
        }
    }
    for &id in &plan.plan.order {
        writer.order(id);
    }
    for (dependency, consumers) in plan.plan.dependents.iter().enumerate() {
        for &consumer in consumers {
            writer.dependent(dependency, consumer);
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
    body
}

/// 即使 check 不触发链接、图没有 provider，也必须拒绝旧版 core 输入协议。
/// 使用已有 options sink 的版本和完整签名握手，不增加运行时回调或读取布局。
pub(super) fn validate_protocol(tcx: TyCtxt<'_>) {
    let available = helpers(tcx);
    let Some(&definition) = available.get(&PlanSink::Options) else {
        tcx.dcx().fatal(
            "Nestrs 工具链与 nestrs-core 内部协议版本不匹配：需要 plan_set_options_v3；请同步更新并重编译工具链与 core",
        );
    };
    let signature = tcx
        .fn_sig(definition)
        .instantiate_identity()
        .skip_normalization()
        .skip_binder();
    let inputs_match = matches!(signature.inputs(), [output, eager, scope_eager, concurrency]
        if matches!(output.kind(), ty::RawPtr(element, mutability)
            if *element == tcx.types.unit && mutability.is_mut())
        && *eager == tcx.types.bool
        && *scope_eager == tcx.types.bool
        && *concurrency == tcx.types.usize);
    if tcx.generics_of(definition).count() != 0
        || signature.safety().is_safe()
        || signature.abi() != ExternAbi::Rust
        || signature.c_variadic()
        || signature.output() != tcx.types.unit
        || !inputs_match
    {
        tcx.dcx().fatal(
            "Nestrs 工具链与 nestrs-core 内部协议签名不匹配：plan_set_options_v3；请同步更新并重编译工具链与 core",
        );
    }
}

/// 按 core 定义路径收集计划写入函数，具体签名在协议验证与调用前核对。
fn helpers(tcx: TyCtxt<'_>) -> HashMap<PlanSink, DefId> {
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
                && let Some(sink) = PlanSink::from_name(name.as_str())
                && definition_path(tcx, def) == format!("graph::plan::{}", sink.name())
            {
                result.insert(sink, def);
            }
        }
    }
    result
}

/// 选择结果使用具名字段传入；只有 Writer 负责现有 core ABI 的参数位置。
struct ProviderEmission<'a, 'tcx> {
    /// 返回真实 ActivationAdapter 的闭合回调。
    callback: ty::Instance<'tcx>,

    /// 已通过全图校验的 provider 声明。
    declaration: &'a Provider<'tcx>,

    /// 该节点的完整激活闭包是否需要 scope。
    requires_scope: bool,
}

/// 一个输入槽位的声明信息与冻结选择结果。
struct InputEmission<'a> {
    /// 消费者在最终 provider 目录中的索引。
    provider: usize,

    /// 消费者内部的输入槽位索引。
    slot: usize,

    /// 已选目标与可选投影，不再在运行期求解。
    selected: &'a model::InputPlan,

    /// 原始请求的 optional、key 与诊断标签。
    declaration: &'a model::Input,
}

/// 按已认证执行 ABI 构造入口 MIR；不调用用户构造函数。
struct Writer<'a, 'tcx> {
    /// 用于构造目标类型、常量及函数实例的编译上下文。
    tcx: TyCtxt<'tcx>,

    /// 将被替换的入口函数体及新增局部变量。
    body: &'a mut mir::Body<'tcx>,

    /// 按调用顺序积累的 MIR 基本块。
    blocks: IndexVec<BasicBlock, BasicBlockData<'tcx>>,

    /// 生成指令共用的入口源码信息。
    info: mir::SourceInfo,

    /// 按定义路径定位的 core 写入函数，调用前仍需核对签名。
    helpers: HashMap<PlanSink, DefId>,
}

impl<'tcx> Writer<'_, 'tcx> {
    /// 把入口 manifest 已确定的初始化策略和并发上限写入计划。
    fn options(&mut self, config: &cargo_nestrs::project_config::DiConfig) {
        self.sink(
            PlanSink::Options,
            vec![
                self.pointer(),
                boolean(self.tcx, config.eager, self.info.span),
                boolean(self.tcx, config.scope_eager, self.info.span),
                number(self.tcx, config.max_concurrent_activations, self.info.span),
            ],
        );
    }

    /// 调用已选 typed projection 描述回调，把返回的 adapter 交给 core。
    fn binding(&mut self, instance: ty::Instance<'tcx>) {
        let value = self.descriptor(instance, protocol::PROJECTION_ADAPTER);
        self.sink(PlanSink::Binding, vec![self.pointer(), value]);
    }

    /// 编码已验证 provider 的生命周期、key、预热策略和用户来源。
    fn provider(&mut self, emission: ProviderEmission<'_, 'tcx>) {
        let value = self.descriptor(emission.callback, protocol::ACTIVATION_ADAPTER);
        let provider = emission.declaration;
        let loc = self
            .tcx
            .sess
            .source_map()
            .lookup_char_pos(provider.source.source_callsite().lo());
        let source_file = loc.file.name.prefer_local_unconditionally().to_string();
        let key = EncodedKey::new(&provider.data.key);
        let lifetime = match provider.data.lifetime {
            model::Lifetime::Singleton => Lifetime::Singleton,
            model::Lifetime::Scoped => Lifetime::Scoped,
            model::Lifetime::Transient => Lifetime::Transient,
        };
        let initialization = Initialization::from_lazy(provider.data.lazy);
        let (tcx, span) = (self.tcx, self.info.span);
        self.sink(
            PlanSink::Provider,
            vec![
                self.pointer(),
                value,
                number(tcx, lifetime as usize, span),
                number(tcx, key.kind as usize, span),
                text(tcx, key.name, span),
                number(tcx, key.index, span),
                number(tcx, initialization as usize, span),
                text(tcx, &source_file, span),
                number(tcx, loc.line, span),
                number(tcx, loc.col.0 + 1, span),
                boolean(tcx, emission.requires_scope, span),
            ],
        );
    }

    /// 编码输入的已选目标与投影，并保留 optional、key 及诊断标签。
    fn input(&mut self, emission: InputEmission<'_>) {
        let declaration = emission.declaration;
        let key = EncodedKey::new(&declaration.key);
        let (tcx, span) = (self.tcx, self.info.span);
        self.sink(
            PlanSink::Input,
            vec![
                self.pointer(),
                number(tcx, emission.provider, span),
                number(tcx, emission.slot, span),
                optional_number(tcx, emission.selected.target, span),
                optional_number(tcx, emission.selected.binding, span),
                boolean(tcx, declaration.optional, span),
                number(tcx, key.kind as usize, span),
                text(tcx, key.name, span),
                number(tcx, key.index, span),
                text(tcx, &declaration.label, span),
            ],
        );
    }

    /// 记录 trait 查询路由使用的 provider 和投影索引。
    fn trait_route(&mut self, provider: usize, binding: usize) {
        self.sink(
            PlanSink::TraitRoute,
            vec![
                self.pointer(),
                number(self.tcx, provider, self.info.span),
                number(self.tcx, binding, self.info.span),
            ],
        );
    }

    /// 追加一个已验证拓扑顺序中的 provider 索引。
    fn order(&mut self, provider: usize) {
        self.sink(
            PlanSink::Order,
            vec![self.pointer(), number(self.tcx, provider, self.info.span)],
        );
    }

    /// 记录依赖指向消费者的反向边，供运行期推进与关闭使用。
    fn dependent(&mut self, dependency: usize, consumer: usize) {
        self.sink(
            PlanSink::Dependent,
            vec![
                self.pointer(),
                number(self.tcx, dependency, self.info.span),
                number(self.tcx, consumer, self.info.span),
            ],
        );
    }

    /// 读取入口的计划装配指针参数，不在编译器宿主解引用它。
    fn pointer(&self) -> Operand<'tcx> {
        Operand::Copy(Local::new(1).into())
    }

    /// 追加一个真实函数调用基本块与返回值局部变量，结果作为下一条调用的操作数。
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

    /// 验证无参数 typed adapter 回调签名，再生成目标端调用。
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
            || !matches!(
                sig.output().kind(),
                ty::Adt(d, _) if self.tcx.crate_name(d.did().krate).as_str() == "nestrs_core"
                    && definition_path(self.tcx, d.did()) == expected
            )
        {
            self.tcx.dcx().fatal("不兼容的 Nestrs typed descriptor ABI");
        }
        self.call(instance, vec![], sig.output())
    }

    /// 逐参数检查 core 写入 ABI 后生成调用；不兼容协议立即终止编译。
    fn sink(&mut self, sink: PlanSink, args: Vec<Operand<'tcx>>) {
        let name = sink.name();
        let def = *self.helpers.get(&sink).unwrap_or_else(|| {
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

/// 为目标 MIR 常量附上当前生成位置。
fn constant(value: mir::Const<'_>, span: Span) -> Operand<'_> {
    Operand::Constant(Box::new(mir::ConstOperand {
        span,
        user_ty: None,
        const_: value,
    }))
}

/// 生成目标 usize 常量，由 rustc 按目标布局表达。
fn number(tcx: TyCtxt<'_>, value: usize, span: Span) -> Operand<'_> {
    constant(mir::Const::from_usize(tcx, value as u64), span)
}

/// 以目标 usize::MAX 编码缺席索引，避免混用宿主指针宽度。
fn optional_number(tcx: TyCtxt<'_>, value: Option<usize>, span: Span) -> Operand<'_> {
    // None使用目标usize::MAX，不能把64位宿主哨兵截断当作类型协议。
    let value = value
        .map(|v| v as u64)
        .unwrap_or_else(|| u64::MAX >> (64 - tcx.sess.target.pointer_width));
    constant(mir::Const::from_usize(tcx, value), span)
}

/// 生成目标 bool 常量操作数。
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

/// 执行 ABI 中 key 的分类与载荷；与审阅 JSON 格式相互独立。
struct EncodedKey<'a> {
    /// 默认、字符串或整数 key 的 ABI 标记。
    kind: KeyKind,

    /// 字符串 key 的内容；其余种类使用空串。
    name: &'a str,

    /// 目标 usize 可表达的整数 key；其余种类使用零。
    index: usize,
}

impl<'a> EncodedKey<'a> {
    /// 按 key 种类拆分 ABI 载荷；整数须已通过可表示范围校验。
    fn new(key: &'a model::Key) -> Self {
        match key {
            model::Key::Default => Self {
                kind: KeyKind::Default,
                name: "",
                index: 0,
            },
            model::Key::Named(value) => Self {
                kind: KeyKind::Named,
                name: value,
                index: 0,
            },
            model::Key::Indexed(value) => Self {
                kind: KeyKind::Indexed,
                name: "",
                index: usize::try_from(*value).expect("compiler key fits target usize"),
            },
        }
    }
}
