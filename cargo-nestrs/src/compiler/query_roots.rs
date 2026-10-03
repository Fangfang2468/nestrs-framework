//! 普通查询方法的编译器语义收集。
//!
//! 查询根属于类型检查后的源码语义，不属于优化后的运行路径。每个函数/闭包的 HIR
//! 摘要分别记录直接查询的 T、引用的函数项类型 F 和常量的真实定义及实参；查询与
//! 函数摘要复制到 MIR 入口，常量引用沿用原生 required_consts，随上游 metadata
//! 保留。因此 `if false`、未调用函数和 Release 消除都不会删掉声明。
//! 泛型函数只在出现闭合调用实例时替换参数；显式队列处理调用链，不执行业务函数。

extern crate rustc_index;
extern crate rustc_infer;
extern crate rustc_trait_selection;

use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, DefIndex, LocalDefId, LocalModDefId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_index::Idx;
use rustc_infer::infer::TyCtxtInferExt;
use rustc_middle::mir::{self, BasicBlock, BasicBlockData, Operand, TerminatorKind};
use rustc_middle::ty::{self, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::{Span, Spanned};
use rustc_trait_selection::traits::{Obligation, ObligationCause, ObligationCtxt};
use std::collections::{HashMap, HashSet, VecDeque};

use crate::registration_codegen::definition_path;

/// 有限编译计划的防护界限。独立的简单类型总数允许一万层以上的普通 DI 图；
/// 对不断增长的 A<Vec<T>> 类递归，先限制单个类型树大小，再归一化/trait 求解，
/// 避免等到分配无限类型或耗尽编译线程栈时才失败。
pub const MAX_QUERY_TYPES: usize = 100_000;

pub fn validate_type_complexity(tcx: TyCtxt<'_>, value: Ty<'_>) -> Result<(), String> {
    let limit = (tcx.recursion_limit().0 * 8).max(1024);
    if value.walk().take(limit + 1).count() > limit {
        return Err(format!(
            "DI 泛型类型不断增长或过于复杂：单个类型树超过 {limit} 个节点；请终止递归泛型查询或拆分类型"
        ));
    }
    Ok(())
}

pub const SUMMARY_NAME: &str = crate::protocol::Marker::QuerySummary.name();

const ROOT_MARKER: &str = crate::protocol::Marker::QueryRoot.name();
const CALL_MARKER: &str = crate::protocol::Marker::QueryCall.name();

#[derive(Clone, Copy)]
struct Record<'tcx> {
    value: QueryValue<'tcx>,
    span: Span,
}

/// 常量引用保留真实定义与实参；FnPtr 只描述签名，不能恢复其初始化器身份。
/// 不把常量伪装成 FnDef，也不求值用户常量来寻找函数地址。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum QueryValue<'tcx> {
    Service(Ty<'tcx>),
    Callable(Ty<'tcx>),
    Constant(DefId, ty::GenericArgsRef<'tcx>),
}

impl<'tcx> QueryValue<'tcx> {
    fn definition(self) -> Option<DefId> {
        match self {
            Self::Service(_) => None,
            Self::Callable(value) => match *value.kind() {
                ty::FnDef(id, _)
                | ty::Closure(id, _)
                | ty::Coroutine(id, _)
                | ty::CoroutineClosure(id, _) => Some(id),
                _ => None,
            },
            Self::Constant(id, _) => Some(id),
        }
    }

    fn closed(self) -> bool {
        match self {
            Self::Service(value) | Self::Callable(value) => closed(value),
            Self::Constant(_, args) => {
                !args.has_non_region_param() && !args.has_infer() && !args.has_escaping_bound_vars()
            }
        }
    }

    fn instantiate(self, tcx: TyCtxt<'tcx>, args: ty::GenericArgsRef<'tcx>) -> Self {
        match self {
            Self::Service(value) => Self::Service(
                ty::EarlyBinder::bind(tcx, value)
                    .instantiate(tcx, args)
                    .skip_normalization(),
            ),
            Self::Callable(value) => Self::Callable(
                ty::EarlyBinder::bind(tcx, value)
                    .instantiate(tcx, args)
                    .skip_normalization(),
            ),
            Self::Constant(id, value) => Self::Constant(
                id,
                ty::EarlyBinder::bind(tcx, value)
                    .instantiate(tcx, args)
                    .skip_normalization(),
            ),
        }
    }

    fn normalize(self, tcx: TyCtxt<'tcx>) -> Result<Self, String> {
        match self {
            Self::Service(value) | Self::Callable(value) => {
                validate_type_complexity(tcx, value)?;
                let normalized = tcx
                    .try_normalize_erasing_regions(
                        ty::TypingEnv::fully_monomorphized(),
                        ty::Unnormalized::new_wip(value),
                    )
                    .map_err(|error| format!("无法归一化 DI 查询类型 {value}: {error:?}"))?;
                Ok(if matches!(self, Self::Service(_)) {
                    Self::Service(normalized)
                } else {
                    Self::Callable(normalized)
                })
            }
            Self::Constant(id, args) => {
                for value in args.types() {
                    validate_type_complexity(tcx, value)?;
                }
                let args = tcx
                    .try_normalize_erasing_regions(
                        ty::TypingEnv::fully_monomorphized(),
                        ty::Unnormalized::new_wip(args),
                    )
                    .map_err(|error| {
                        format!("无法归一化 DI 查询常量 {}: {error:?}", tcx.def_path_str(id))
                    })?;
                Ok(Self::Constant(id, args))
            }
        }
    }
}

/// 供自动 binding 和最终计划共同消费的闭合查询根。类型身份始终是 rustc Ty，
/// span 仅用于诊断；不能用类型的显示字符串去去重或重新推导类型。
#[derive(Clone, Copy)]
pub struct QueryRoot<'tcx> {
    pub service: Ty<'tcx>,
    pub owner: LocalDefId,
    pub span: Span,
}

fn query_method(tcx: TyCtxt<'_>, method: DefId) -> bool {
    if tcx.crate_name(method.krate).as_str() != "nestrs_core"
        || !tcx.opt_item_name(method).is_some_and(|name| {
            matches!(
                name.as_str(),
                "get_required_service"
                    | "get_service"
                    | "get_required_keyed_service"
                    | "get_keyed_service"
            )
        })
    {
        return false;
    }
    let Some(implementation) = tcx.impl_of_assoc(method) else {
        return false;
    };
    let owner = tcx
        .type_of(implementation)
        .instantiate_identity()
        .skip_normalization();
    let ty::Adt(definition, _) = owner.kind() else {
        return false;
    };
    matches!(
        definition_path(tcx, definition.did()).as_str(),
        "facade::ServiceProvider" | "facade::ServiceProviderRef"
    )
}

fn business_definition(tcx: TyCtxt<'_>, id: DefId) -> bool {
    match tcx.crate_name(id.krate).as_str() {
        "core" | "alloc" | "std" => false,
        // core 内部 ABI 回归把真实服务声明放在本 crate 的 test 模块，必须与应用
        // 获得相同查询收集语义；普通 runtime rlib 则绝不作为业务调用图展开。
        "nestrs_core" => id.is_local() && tcx.sess.opts.test,
        _ => true,
    }
}

struct Summary<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    typeck: &'tcx ty::TypeckResults<'tcx>,
    records: &'a mut Vec<Record<'tcx>>,
}
impl<'tcx> Summary<'_, 'tcx> {
    fn function(&mut self, id: DefId, args: ty::GenericArgsRef<'tcx>, span: Span) {
        if query_method(self.tcx, id) {
            if let Some(service) = args.types().last() {
                self.records.push(Record {
                    value: QueryValue::Service(self.tcx.erase_and_anonymize_regions(service)),
                    span,
                });
            }
        } else if business_definition(self.tcx, id)
            || (self.tcx.def_kind(id) == DefKind::AssocFn && self.tcx.trait_of_assoc(id).is_some())
        {
            // 调用点可以引用标准库的 trait 方法，而实际实现位于业务 crate。
            // 先保存带实参的 trait 方法身份，待闭合后由 Instance 求解实现；不能
            // 按 trait 所属 crate 提前丢弃 Iterator::next、Add::add 等调用。
            // 这里只保存调用边，函数体仍仅从业务摘要读取，不扫描标准库 MIR。
            self.records.push(Record {
                value: QueryValue::Callable(
                    self.tcx
                        .erase_and_anonymize_regions(Ty::new_fn_def(self.tcx, id, args)),
                ),
                span,
            });
        }
    }
}
impl<'tcx> Visitor<'tcx> for Summary<'_, 'tcx> {
    fn visit_expr(&mut self, expression: &'tcx rustc_hir::Expr<'tcx>) {
        // 运算符、索引等重载表达式也有经过类型检查的关联方法身份。它们与普通
        // MethodCall 使用同一闭合实例求解；只匹配 MethodCall 会漏掉 `a + b`。
        // 同一表还保存 Self::CONST 等关联常量，只有关联函数可以编码成 FnDef。
        if let Some(id) = self.typeck.type_dependent_def_id(expression.hir_id)
            && self.tcx.def_kind(id) == DefKind::AssocFn
        {
            self.function(
                id,
                self.typeck.node_args(expression.hir_id),
                expression.span,
            );
        }
        if let rustc_hir::ExprKind::Path(ref path) = expression.kind
            && let Res::Def(DefKind::Const { .. } | DefKind::AssocConst { .. }, id) =
                self.typeck.qpath_res(path, expression.hir_id)
            && business_definition(self.tcx, id)
        {
            self.records.push(Record {
                value: QueryValue::Constant(
                    id,
                    self.tcx
                        .erase_and_anonymize_regions(self.typeck.node_args(expression.hir_id)),
                ),
                span: expression.span,
            });
        }
        if let rustc_hir::ExprKind::ConstBlock(block) = expression.kind {
            // inline const 有独立 body 和合成类型参数；保留与 rustc THIR 相同的
            // 实参布局，才能从外层泛型函数继续闭合初始化器中的关联常量。
            let parent_args =
                self.tcx
                    .erase_and_anonymize_regions(ty::GenericArgs::identity_for_item(
                        self.tcx,
                        self.tcx.typeck_root_def_id_local(block.def_id),
                    ));
            let args = ty::InlineConstArgs::new(
                self.tcx,
                ty::InlineConstArgsParts {
                    parent_args,
                    ty: self.typeck.node_type(block.hir_id),
                },
            )
            .args;
            self.records.push(Record {
                value: QueryValue::Constant(block.def_id.to_def_id(), args),
                span: expression.span,
            });
        }
        match *self.typeck.expr_ty(expression).kind() {
            ty::FnDef(id, args) => self.function(id, args, expression.span),
            ty::Closure(id, _) | ty::Coroutine(id, _) | ty::CoroutineClosure(id, _)
                if matches!(expression.kind, rustc_hir::ExprKind::Closure(..))
                    && business_definition(self.tcx, id) =>
            {
                self.records.push(Record {
                    value: QueryValue::Callable(
                        self.tcx
                            .erase_and_anonymize_regions(self.typeck.expr_ty(expression)),
                    ),
                    span: expression.span,
                });
            }
            _ => {}
        }
        intravisit::walk_expr(self, expression);
    }
}

fn local_summary<'tcx>(tcx: TyCtxt<'tcx>, owner: LocalDefId) -> Vec<Record<'tcx>> {
    if !business_definition(tcx, owner.to_def_id()) || !tcx.has_typeck_results(owner) {
        return Vec::new();
    }
    let mut records = Vec::new();
    Summary {
        tcx,
        typeck: tcx.typeck(owner),
        records: &mut records,
    }
    .visit_body(tcx.hir_body_owned_by(owner));
    let mut seen = HashSet::new();
    records.retain(|record| seen.insert(record.value));
    records
}

/// 在原始 MIR 完成后加上类型摘要。复制的是已有、通过类型检查的类型身份；既不
/// 改写用户调用，也不伪造借用、trait 投影或任意函数签名。标记函数无参数且为 const。
/// 摘要放在所有分支之前，确保其跨 crate metadata 与 Debug/Release 含义一致。
pub fn preserve_summary<'tcx>(tcx: TyCtxt<'tcx>, owner: LocalDefId, body: &mut mir::Body<'tcx>) {
    // 常量引用由 rustc 原生 required_consts 在优化前保留，随 MIR metadata 编码。
    // 这里只追加原有类型标记；不插入常量求值或运行期调用，也不改变常量有效性规则。
    let records: Vec<_> = local_summary(tcx, owner)
        .into_iter()
        .filter(|record| !matches!(record.value, QueryValue::Constant(..)))
        .collect();
    if records.is_empty() {
        return;
    }
    let Some(root) = crate::reflection::local_marker(tcx, ROOT_MARKER) else {
        return;
    };
    let Some(call) = crate::reflection::local_marker(tcx, CALL_MARKER) else {
        return;
    };
    let source_info = body.basic_blocks[mir::START_BLOCK].terminator().source_info;
    let unit = body
        .local_decls
        .push(mir::LocalDecl::new(tcx.types.unit, source_info.span));
    let old_start = BasicBlock::new(body.basic_blocks.len());
    let start = body.basic_blocks[mir::START_BLOCK].clone();
    let blocks = body.basic_blocks.as_mut();
    blocks.push(start);
    for block in blocks.iter_mut().skip(1) {
        block.terminator_mut().successors_mut(|target| {
            if *target == mir::START_BLOCK {
                *target = old_start;
            }
        });
    }
    let first = BasicBlock::new(blocks.len());
    blocks[mir::START_BLOCK] = BasicBlockData::new(
        Some(mir::Terminator {
            source_info,
            kind: TerminatorKind::Goto { target: first },
            attributes: Default::default(),
        }),
        false,
    );
    let count = records.len();
    for (index, record) in records.into_iter().enumerate() {
        let (function, argument) = match record.value {
            QueryValue::Service(value) => (root, value),
            QueryValue::Callable(value) => (call, value),
            QueryValue::Constant(..) => unreachable!("常量引用使用原生 MIR metadata"),
        };
        let next = if index + 1 == count {
            old_start
        } else {
            BasicBlock::new(blocks.len() + 1)
        };
        blocks.push(call_block(
            tcx,
            function,
            argument,
            unit.into(),
            next,
            mir::SourceInfo {
                span: record.span,
                ..source_info
            },
        ));
    }
}

fn call_block<'tcx>(
    tcx: TyCtxt<'tcx>,
    function: DefId,
    argument: Ty<'tcx>,
    destination: mir::Place<'tcx>,
    target: BasicBlock,
    source_info: mir::SourceInfo,
) -> BasicBlockData<'tcx> {
    BasicBlockData::new(
        Some(mir::Terminator {
            source_info,
            kind: TerminatorKind::Call {
                func: Operand::Constant(Box::new(mir::ConstOperand {
                    span: source_info.span,
                    user_ty: None,
                    const_: mir::Const::zero_sized(Ty::new_fn_def(tcx, function, [argument])),
                })),
                args: Vec::<Spanned<Operand<'tcx>>>::new().into(),
                destination,
                target: Some(target),
                unwind: mir::UnwindAction::Continue,
                call_source: mir::CallSource::Misc,
                fn_span: source_info.span,
            },
            attributes: Default::default(),
        }),
        false,
    )
}

fn external_summary<'tcx>(tcx: TyCtxt<'tcx>, id: DefId) -> Vec<Record<'tcx>> {
    // 常量初始化器随 CTFE MIR 发布；is_mir_available/optimized_mir 只覆盖函数侧。
    // 读取 MIR 不等于求值常量，其函数指针、链式常量和开放 trait 实参仍保留身份。
    let body = if matches!(
        tcx.def_kind(id),
        DefKind::Const { .. } | DefKind::AssocConst { .. } | DefKind::InlineConst
    ) {
        if !tcx.defaultness(id).has_value() || tcx.trivial_const(id).is_some() {
            return Vec::new();
        }
        tcx.mir_for_ctfe(id)
    } else {
        if !tcx.is_mir_available(id) {
            return Vec::new();
        }
        tcx.optimized_mir(id)
    };
    let mut records: Vec<_> = body
        .basic_blocks
        .iter()
        .filter_map(|block| {
            let TerminatorKind::Call { func, .. } = &block.terminator().kind else {
                return None;
            };
            let ty::FnDef(callee, args) = *func.ty(&body.local_decls, tcx).kind() else {
                return None;
            };
            let value = if crate::registration_codegen::reflect_item(tcx, callee, ROOT_MARKER) {
                QueryValue::Service(args.type_at(0))
            } else if crate::registration_codegen::reflect_item(tcx, callee, CALL_MARKER) {
                QueryValue::Callable(args.type_at(0))
            } else {
                return None;
            };
            Some(Record {
                value,
                span: block.terminator().source_info.span,
            })
        })
        .collect();
    // rustc 在 promotion 和优化之前记录此列表；if false 中被消除的常量引用也在。
    // 不读取已求值函数地址，不遍历分配，也不把常量签名误当成函数项。
    for constant in body.required_consts.iter().flatten() {
        if let mir::Const::Unevaluated(value, _) = constant.const_
            && value.promoted.is_none()
            && business_definition(tcx, value.def)
            && matches!(
                tcx.def_kind(value.def),
                DefKind::Const { .. } | DefKind::AssocConst { .. } | DefKind::InlineConst
            )
        {
            records.push(Record {
                value: QueryValue::Constant(value.def, value.args),
                span: constant.span,
            });
        }
    }
    records
}

fn closed(value: Ty<'_>) -> bool {
    !value.has_non_region_param() && !value.has_infer() && !value.has_escaping_bound_vars()
}

/// 每个编译入口汇总全部已类型检查的 body。非泛型 body 即使没有被调用也贡献查询；
/// 泛型 body 的闭合查询同样直接贡献，带参数的查询则等待实际闭合函数项进行替换。
/// 只沿保存的语义摘要遍历，不沿优化后的控制流可达性遍历。
pub fn collect<'tcx>(tcx: TyCtxt<'tcx>) -> Result<Vec<QueryRoot<'tcx>>, String> {
    collect_with_providers(tcx, &[])
}

/// 已知闭合服务也是方法 Self 的有限类型来源。它们可能只从字段依赖进入图，随后
/// 通过 trait object 调用业务方法；这时业务调用表达式没有可直接单态化的函数项。
/// 将已知 concrete 与真实 impl Self 统一后，相关方法摘要仍可精确替换参数。
/// 方法自带且没有调用点固定的额外类型/const 参数不会被猜测或枚举。
pub fn collect_with_providers<'tcx>(
    tcx: TyCtxt<'tcx>,
    providers: &[Ty<'tcx>],
) -> Result<Vec<QueryRoot<'tcx>>, String> {
    let mut summaries = HashMap::new();
    let mut processed_crates = HashSet::new();
    for owner in tcx.hir_body_owners() {
        let records = local_summary(tcx, owner);
        if !records.is_empty() {
            summaries.insert(owner.to_def_id(), records);
        }
    }
    for &krate in tcx.crates(()) {
        if !business_definition(
            tcx,
            DefId {
                krate,
                index: DefIndex::from_usize(0),
            },
        ) {
            continue;
        }
        // 只解码经过 Nestrs 摘要生产管线的 crate。版本标记本身不调用业务代码，
        // tokio/serde 等普通依赖不会承担逐函数 MIR 解码成本。
        let processed = (0..tcx.num_extern_def_ids(krate)).any(|index| {
            let id = DefId {
                krate,
                index: DefIndex::from_usize(index),
            };
            tcx.is_mir_available(id) && crate::reflection::summary_definition(tcx, id)
        });
        if !processed {
            continue;
        }
        processed_crates.insert(krate);
        for index in 0..tcx.num_extern_def_ids(krate) {
            let id = DefId {
                krate,
                index: DefIndex::from_usize(index),
            };
            if !tcx.is_mir_available(id) {
                continue;
            }
            let records = external_summary(tcx, id);
            if !records.is_empty() {
                summaries.insert(id, records);
            }
        }
    }
    // 函数 metadata 表不包含常量 CTFE MIR。沿真实常量引用按需加载初始化器，
    // 不对 metadata 的空洞调用 def_kind，也不扫描或求值任意依赖库的常量。
    // trait 常量的实现摘要只为反向可达分析提供候选；闭合后仍由 rustc 精确选定。
    let mut constants: VecDeque<_> = summaries
        .values()
        .flatten()
        .filter_map(|record| {
            if let QueryValue::Constant(id, _) = record.value {
                Some(id)
            } else {
                None
            }
        })
        .collect();
    let mut seen_constants = HashSet::new();
    while let Some(id) = constants.pop_front() {
        if !seen_constants.insert(id) {
            continue;
        }
        if let Some(trait_id) = tcx.trait_of_assoc(id) {
            for implementation in tcx.all_impls(trait_id) {
                if let Some(&item) = tcx.impl_item_implementor_ids(implementation).get(&id) {
                    constants.push_back(item);
                }
            }
        }
        if id.is_local() || !processed_crates.contains(&id.krate) {
            continue;
        }
        let records = external_summary(tcx, id);
        for record in &records {
            if let QueryValue::Constant(id, _) = record.value {
                constants.push_back(id);
            }
        }
        if !records.is_empty() {
            summaries.insert(id, records);
        }
    }
    // 在泛型替换之前计算摘要的反向可达集合，避免与 DI 无关的递归泛型函数触发
    // 查询展开限制。trait 调用先关联真实 impl 对应的 trait item，闭合时仍由 rustc
    // Instance 求解实际实现；这个粗筛只排除不可能包含查询的函数，不选择实现。
    let mut incoming: HashMap<DefId, Vec<DefId>> = HashMap::new();
    let mut relevant = HashSet::new();
    let mut propagation = VecDeque::new();
    for (&id, records) in &summaries {
        if records
            .iter()
            .any(|record| matches!(record.value, QueryValue::Service(_)))
        {
            propagation.push_back(id);
        }
        for record in records {
            if let Some(target) = record.value.definition() {
                incoming.entry(target).or_default().push(id);
            }
        }
        if matches!(
            tcx.def_kind(id),
            DefKind::AssocFn | DefKind::AssocConst { .. }
        ) && let Some(trait_item) = tcx.associated_item(id).trait_item_def_id()
        {
            incoming.entry(id).or_default().push(trait_item);
        }
    }
    while let Some(id) = propagation.pop_front() {
        if relevant.insert(id) {
            propagation.extend(incoming.get(&id).into_iter().flatten().copied());
        }
    }
    let relevant_record = |record: &Record<'tcx>| {
        matches!(record.value, QueryValue::Service(_))
            || record
                .value
                .definition()
                .is_some_and(|id| relevant.contains(&id))
    };
    let fallback = LocalModDefId::CRATE_DEF_ID.to_local_def_id();
    let mut pending = VecDeque::new();
    for (&id, records) in &summaries {
        let owner = id.as_local().unwrap_or(fallback);
        for &record in records {
            if relevant_record(&record) && record.value.closed() {
                pending.push_back((record, owner, 0usize));
            }
        }
    }
    for (&method, records) in &summaries {
        if !relevant.contains(&method) || tcx.def_kind(method) != DefKind::AssocFn {
            continue;
        }
        if tcx
            .generics_of(method)
            .own_params
            .iter()
            .any(|parameter| !matches!(parameter.kind, ty::GenericParamDefKind::Lifetime))
        {
            continue;
        }
        for &provider in providers {
            for args in provider_method_arguments(tcx, method, provider) {
                let owner = method.as_local().unwrap_or(fallback);
                for &record in records.iter().filter(|record| relevant_record(record)) {
                    let value = record.value.instantiate(tcx, args);
                    pending.push_back((Record { value, ..record }, owner, 0));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut seen_roots = HashSet::new();
    let mut roots = Vec::new();
    let mut count = 0usize;
    while let Some((mut record, owner, depth)) = pending.pop_front() {
        count += 1;
        if count > MAX_QUERY_TYPES {
            return Err(
                "DI 查询泛型实例展开超过有限分析上限，可能存在不断增长的递归泛型调用".into(),
            );
        }
        record.value = record.value.normalize(tcx)?;
        if !record.value.closed() {
            continue;
        }
        if let QueryValue::Service(service) = record.value {
            if seen_roots.insert(service) {
                roots.push(QueryRoot {
                    service,
                    owner,
                    span: record.span,
                });
            }
            continue;
        }
        let (id, args) = match record.value {
            QueryValue::Callable(value) => match *value.kind() {
                ty::FnDef(id, args) => (id, args),
                ty::Closure(id, args)
                | ty::Coroutine(id, args)
                | ty::CoroutineClosure(id, args) => (id, args),
                _ => continue,
            },
            QueryValue::Constant(id, args) => (id, args),
            QueryValue::Service(_) => unreachable!(),
        };
        let (id, args) = if matches!(
            tcx.def_kind(id),
            DefKind::Closure | DefKind::SyntheticCoroutineBody
        ) {
            (id, args)
        } else {
            match ty::Instance::try_resolve(tcx, ty::TypingEnv::fully_monomorphized(), id, args)
                .map_err(|_| format!("无法解析 DI 查询 helper {}", tcx.def_path_str(id)))?
            {
                // dyn 调用的 Self 是 trait object，不是实际实现。默认方法也
                // 不能以 dyn Self 物化；实际 concrete 由已知 provider seed 决定。
                Some(instance) if !matches!(instance.def, ty::InstanceKind::Virtual(..)) => {
                    (instance.def_id(), instance.args)
                }
                Some(_) => continue,
                None => continue,
            }
        };
        if !seen.insert((id, args)) {
            continue;
        }
        let Some(records) = summaries.get(&id) else {
            continue;
        };
        for &nested in records.iter().filter(|record| relevant_record(record)) {
            let value = nested.value.instantiate(tcx, args);
            pending.push_back((Record { value, ..nested }, owner, depth + 1));
        }
    }
    roots.sort_by_key(|root| (root.service.to_string(), format!("{:?}", root.service)));
    Ok(roots)
}

/// 对 inherent/显式 trait 方法直接统一 impl Self；对继承的 trait 默认方法，先
/// 统一真实 impl，再把其 trait 参数投影到默认方法。默认方法被 override 时不贡献
/// 已经失效的默认 body，额外 where 条件无法成立的方法也不会制造错误查询根。
fn provider_method_arguments<'tcx>(
    tcx: TyCtxt<'tcx>,
    method: DefId,
    provider: Ty<'tcx>,
) -> Vec<ty::GenericArgsRef<'tcx>> {
    let implementation = tcx.impl_of_assoc(method);
    let candidates: Vec<_> = if let Some(implementation) = implementation {
        vec![implementation]
    } else if let Some(trait_id) = tcx.trait_of_assoc(method) {
        tcx.all_impls(trait_id).collect()
    } else {
        return Vec::new();
    };
    let mut instances = Vec::new();
    for candidate in candidates {
        let infcx = tcx
            .infer_ctxt()
            .ignoring_regions()
            .build(ty::TypingMode::non_body_analysis());
        let args = infcx.fresh_args_for_item(tcx.def_span(candidate), candidate);
        let ocx = ObligationCtxt::new(&infcx);
        let cause = ObligationCause::dummy();
        let environment = ty::ParamEnv::empty();
        let self_ty = ocx.normalize(
            &cause,
            environment,
            tcx.type_of(candidate).instantiate(tcx, args),
        );
        if ocx.eq(&cause, environment, provider, self_ty).is_err() {
            continue;
        }
        for (predicate, _) in tcx.predicates_of(candidate).instantiate(tcx, args) {
            let predicate = ocx.normalize(&cause, environment, predicate);
            ocx.register_obligation(Obligation::new(tcx, cause.clone(), environment, predicate));
        }
        let args = if implementation.is_some() {
            args.extend_to(tcx, method, |_, _| tcx.lifetimes.re_erased.into())
        } else {
            let reference = ocx.normalize(
                &cause,
                environment,
                tcx.impl_trait_ref(candidate).instantiate(tcx, args),
            );
            reference
                .args
                .extend_to(tcx, method, |_, _| tcx.lifetimes.re_erased.into())
        };
        for (predicate, _) in tcx.predicates_of(method).instantiate(tcx, args) {
            let predicate = ocx.normalize(&cause, environment, predicate);
            ocx.register_obligation(Obligation::new(tcx, cause.clone(), environment, predicate));
        }
        if !ocx.evaluate_obligations_error_on_ambiguity().is_empty() {
            continue;
        }
        let args = tcx.erase_and_anonymize_regions(infcx.resolve_vars_if_possible(args));
        if args.has_infer() || args.has_non_region_param() || args.has_escaping_bound_vars() {
            continue;
        }
        if implementation.is_none()
            && !matches!(ty::Instance::try_resolve(tcx, ty::TypingEnv::fully_monomorphized(), method, args),
            Ok(Some(instance)) if instance.def_id() == method)
        {
            continue;
        }
        instances.push(args);
    }
    instances
}
