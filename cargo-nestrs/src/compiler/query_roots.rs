//! 普通查询方法的编译器语义收集。
//!
//! 查询根属于类型检查后的源码语义，不属于优化后的运行路径。每个函数/闭包的 HIR
//! 摘要分别记录直接查询的 T、引用的函数项类型 F 和常量的真实定义及实参；查询与
//! 函数摘要复制到 MIR 入口，不可变 static 的闭合摘要汇入私有 summary 函数，常量
//! 引用沿用原生 required_consts。因此 `if false`、未调用函数和 Release 消除都
//! 不会删掉业务声明。标准库转发只按携带已知查询类型的闭合调用读取其原生 MIR；
//! 泛型替换和调用展开使用显式队列，不执行业务函数，也不求值静态函数指针。

extern crate rustc_index;
extern crate rustc_infer;
extern crate rustc_trait_selection;

use rustc_hir::LangItem;
use rustc_hir::def::{DefKind, Res};
use rustc_hir::def_id::{DefId, DefIndex, LocalDefId, LocalModDefId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_index::Idx;
use rustc_infer::infer::TyCtxtInferExt;
use rustc_middle::mir::{self, BasicBlock, BasicBlockData, Operand, TerminatorKind};
use rustc_middle::ty::adjustment::{Adjust, DerefAdjustKind};
use rustc_middle::ty::{self, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::{Span, Spanned};
use rustc_trait_selection::traits::{Obligation, ObligationCause, ObligationCtxt};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::registration_codegen::definition_path;

/// 有限编译计划的防护界限。独立的简单类型总数允许一万层以上的普通 DI 图；
/// 对不断增长的 A<Vec<T>> 类递归，先限制单个类型树大小，再归一化/trait 求解，
/// 避免等到分配无限类型或耗尽编译线程栈时才失败。
pub const MAX_QUERY_TYPES: usize = 100_000;

pub(crate) fn validate_type_complexity_at(
    tcx: TyCtxt<'_>,
    value: Ty<'_>,
    span: Span,
    origin: Span,
) -> Result<(), String> {
    let limit = (tcx.recursion_limit().0 * 8).max(1024);
    if value.walk().take(limit + 1).count() > limit {
        expansion_error(
            tcx,
            span,
            origin,
            "single_type_tree_nodes",
            limit,
            format!(
                "DI 泛型类型不断增长或过于复杂：单个类型树超过 {limit} 个节点；请终止递归泛型查询或拆分类型"
            ),
        );
    }
    Ok(())
}

pub(crate) fn expansion_error(
    tcx: TyCtxt<'_>,
    span: Span,
    origin: Span,
    metric: &str,
    limit: usize,
    detail: String,
) -> ! {
    let mut diagnostic = crate::diagnostics::Diagnostic::new(
        "NESTRS-DI008",
        "服务类型的泛型展开超过分析上限".into(),
        span,
    );
    diagnostic.labels.push((
        span,
        "此处引入的类型或调用需要继续展开过于复杂的泛型类型".into(),
    ));
    if origin != span && !origin.is_dummy() {
        diagnostic
            .labels
            .push((origin, "此处提供这条展开链的闭合起点".into()));
    }
    diagnostic.notes.push("已编译的查询与调用都参与类型分析，即使它们尚未执行或位于 if false 分支。此上限按类型复杂度或实例数量计数，不是普通依赖链深度。".into());
    diagnostic.help.push("检查是否有每次递归都改变类型参数的调用；使用固定的有限类型集合，或用运行期数据结构表达递归层次。".into());
    diagnostic.cause = format!("TypeExpansionLimit\nmetric={metric}; limit={limit}\n{detail}");
    crate::diagnostics::emit(tcx, vec![diagnostic]);
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

    fn normalize(self, tcx: TyCtxt<'tcx>, span: Span, origin: Span) -> Result<Self, String> {
        match self {
            Self::Service(value) | Self::Callable(value) => {
                validate_type_complexity_at(tcx, value, span, origin)?;
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
                    validate_type_complexity_at(tcx, value, span, origin)?;
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

fn standard_definition(tcx: TyCtxt<'_>, id: DefId) -> bool {
    matches!(tcx.crate_name(id.krate).as_str(), "core" | "alloc" | "std")
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
            || (standard_definition(self.tcx, id) && args.types().next().is_some())
            || (self.tcx.def_kind(id) == DefKind::AssocFn && self.tcx.trait_of_assoc(id).is_some())
        {
            // 调用点可以引用标准库的 trait 方法，而实际实现位于业务 crate。
            // 先保存带实参的 trait 方法身份，待闭合后由 Instance 求解实现；不能
            // 按 trait 所属 crate 提前丢弃 Iterator::next、Add::add 等调用。
            // 标准库的默认方法也可能继续转发到业务实现。这里只保存真实函数项；
            // 闭合实参携带已知查询类型时，收集阶段才按需读取其 MIR 调用边。
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
        // 自动解引用不占用 type_dependent_def_id。沿原生 adjustment 顺序恢复
        // 每一步的真实接收类型，使用与 THIR 相同的 Deref/DerefMut 方法身份与实参。
        let mut receiver = self.typeck.expr_ty(expression);
        for adjustment in self.typeck.expr_adjustments(expression) {
            if let Adjust::Deref(DerefAdjustKind::Overloaded(deref)) = adjustment.kind {
                self.function(
                    deref.method_call(self.tcx),
                    self.tcx.mk_args(&[receiver.into()]),
                    deref.span,
                );
            }
            receiver = adjustment.target;
        }
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

/// rustc 不把 static 初始化器的 CTFE MIR 编码到上游 metadata。不可变 static
/// 的初始化器已经是闭合、经过类型检查的 body；把其中的函数项摘要放入现有私有
/// summary 函数，才能让下游与本 crate 一样恢复这些查询。只展开常量的定义摘要，
/// 不求值 static/const、不读取函数地址，也不为静态变量创建可调用的伪 FnDef。
fn immutable_static_summary<'tcx>(tcx: TyCtxt<'tcx>) -> Result<Vec<Record<'tcx>>, String> {
    let mut pending = VecDeque::new();
    for owner in tcx.hir_body_owners() {
        if matches!(
            tcx.def_kind(owner),
            DefKind::Static {
                mutability: rustc_hir::Mutability::Not,
                ..
            }
        ) {
            pending.extend(local_summary(tcx, owner));
        }
    }
    let mut seen = HashSet::new();
    let mut records = Vec::new();
    let mut constants = 0usize;
    while let Some(record) = pending.pop_front() {
        // 这里还不能知道 static 是否包含 DI 查询。先保留真实函数项和常量摘要，
        // 相关性分析后才实施查询类型复杂度上限；无关常量的大而浅类型同样合法。
        if !record.value.closed() || !seen.insert(record.value) {
            continue;
        }
        let QueryValue::Constant(id, args) = record.value else {
            records.push(record);
            continue;
        };
        constants += 1;
        if constants > MAX_QUERY_TYPES {
            expansion_error(
                tcx,
                record.span,
                record.span,
                "static_query_instances",
                MAX_QUERY_TYPES,
                "不可变 static 的查询常量摘要超过有限分析上限".into(),
            );
        }
        // 固定定义的常量直接代入摘要即可；trait 常量仍必须让 rustc 选择真实实现。
        // 归一化用于原生常量身份解析，不在这里套用尚无查询需求的 DI 类型上限。
        let (id, args) = if tcx.trait_of_assoc(id).is_some() {
            let args = tcx
                .try_normalize_erasing_regions(
                    ty::TypingEnv::fully_monomorphized(),
                    ty::Unnormalized::new_wip(args),
                )
                .map_err(|error| {
                    format!(
                        "无法归一化 static 查询常量 {}: {error:?}",
                        tcx.def_path_str(id)
                    )
                })?;
            let Some(instance) =
                ty::Instance::try_resolve(tcx, ty::TypingEnv::fully_monomorphized(), id, args)
                    .map_err(|_| format!("无法解析 static 查询常量 {}", tcx.def_path_str(id)))?
            else {
                continue;
            };
            (instance.def_id(), instance.args)
        } else {
            (id, args)
        };
        let nested = if let Some(owner) = id.as_local() {
            local_summary(tcx, owner)
        } else {
            external_summary(tcx, id)
        };
        for nested in nested {
            pending.push_back(Record {
                value: nested.value.instantiate(tcx, args),
                ..nested
            });
        }
    }
    Ok(records)
}

/// 在原始 MIR 完成后加上类型摘要。复制的是已有、通过类型检查的类型身份；既不
/// 改写用户调用，也不伪造借用、trait 投影或任意函数签名。标记函数无参数且为 const。
/// 摘要放在所有分支之前，确保其跨 crate metadata 与 Debug/Release 含义一致。
pub fn preserve_summary<'tcx>(tcx: TyCtxt<'tcx>, owner: LocalDefId, body: &mut mir::Body<'tcx>) {
    // 常量引用由 rustc 原生 required_consts 在优化前保留，随 MIR metadata 编码。
    // 这里只追加原有类型标记；不插入常量求值或运行期调用，也不改变常量有效性规则。
    let summary = if crate::reflection::summary_definition(tcx, owner.to_def_id()) {
        immutable_static_summary(tcx)
            .unwrap_or_else(|error| crate::diagnostics::internal(tcx, error))
    } else {
        local_summary(tcx, owner)
    };
    preserve_records(tcx, body, summary);
}

/// 仅在原生 drop elaboration 完成后保存析构边；move、forget 和 ManuallyDrop
/// 遵守 rustc 自己的析构规则。此阶段尚未优化 if false 的运行路径。
pub fn preserve_drop_summary<'tcx>(tcx: TyCtxt<'tcx>, body: &mut mir::Body<'tcx>) {
    preserve_records(tcx, body, drop_summary(tcx, body));
}

fn preserve_records<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mut mir::Body<'tcx>,
    summary: Vec<Record<'tcx>>,
) {
    let records: Vec<_> = summary
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
    // 运行期 MIR 已完成 unwind lowering；工具生成的空 marker 不会 unwind。
    let unwind = if matches!(body.phase, mir::MirPhase::Runtime(_)) {
        mir::UnwindAction::Unreachable
    } else {
        mir::UnwindAction::Continue
    };
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
            unwind,
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
    unwind: mir::UnwindAction,
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
                unwind,
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

/// 标准库没有 Nestrs marker，但其泛型 MIR 保留了真实函数项与 trait 调用。
/// 只在闭合调用的实参携带已知查询实现/闭包时读取，不扫描整套标准库；函数指针
/// 转换前的 FnDef 同样保留，不能只看 Call terminator 丢掉传递给回调的函数项。
fn drop_callable<'tcx>(tcx: TyCtxt<'tcx>, value: Ty<'tcx>, span: Span) -> Record<'tcx> {
    let method = tcx.require_lang_item(LangItem::DropGlue, span);
    Record {
        value: QueryValue::Callable(tcx.erase_and_anonymize_regions(Ty::new_fn_def(
            tcx,
            method,
            [value],
        ))),
        span,
    }
}

fn drop_type<'tcx>(tcx: TyCtxt<'tcx>, value: QueryValue<'tcx>) -> Option<Ty<'tcx>> {
    if let QueryValue::Callable(value) = value
        && let ty::FnDef(id, args) = *value.kind()
        && Some(id) == tcx.lang_items().drop_glue_fn()
    {
        Some(args.type_at(0))
    } else {
        None
    }
}

fn drop_summary<'tcx>(tcx: TyCtxt<'tcx>, body: &mir::Body<'tcx>) -> Vec<Record<'tcx>> {
    body.basic_blocks
        .iter()
        .filter_map(|block| {
            let terminator = block.terminator();
            let TerminatorKind::Drop { place, .. } = terminator.kind else {
                return None;
            };
            Some(drop_callable(
                tcx,
                place.ty(&body.local_decls, tcx).ty,
                terminator.source_info.span,
            ))
        })
        .collect()
}

fn mir_summary<'tcx>(tcx: TyCtxt<'tcx>, body: &mir::Body<'tcx>) -> Vec<Record<'tcx>> {
    struct Calls<'a, 'tcx> {
        tcx: TyCtxt<'tcx>,
        body: &'a mir::Body<'tcx>,
        records: Vec<Record<'tcx>>,
    }
    impl<'tcx> mir::visit::Visitor<'tcx> for Calls<'_, 'tcx> {
        fn visit_operand(&mut self, operand: &Operand<'tcx>, location: mir::Location) {
            let value = operand.ty(&self.body.local_decls, self.tcx);
            if matches!(
                value.kind(),
                ty::FnDef(..) | ty::Closure(..) | ty::Coroutine(..) | ty::CoroutineClosure(..)
            ) {
                self.records.push(Record {
                    value: QueryValue::Callable(self.tcx.erase_and_anonymize_regions(value)),
                    span: self.body.source_info(location).span,
                });
            }
            self.super_operand(operand, location);
        }
    }
    let mut calls = Calls {
        tcx,
        body,
        records: drop_summary(tcx, body),
    };
    mir::visit::Visitor::visit_body(&mut calls, body);
    let mut seen = HashSet::new();
    calls.records.retain(|record| seen.insert(record.value));
    calls.records
}

fn standard_summary<'tcx>(tcx: TyCtxt<'tcx>, id: DefId) -> Vec<Record<'tcx>> {
    if tcx.is_mir_available(id) {
        mir_summary(tcx, tcx.optimized_mir(id))
    } else {
        Vec::new()
    }
}

/// 只使用已有查询摘要的真实身份作为转发入口：方法所属的名义类型，以及已知
/// 查询函数/闭包。例如 collect<Query<T>> 的 Query 来自业务 next 的 impl Self，
/// 不靠 collect/next 拼写，也不枚举其他 impl 或猜测泛型实参。这个筛选只决定是否
/// 读取调用边；实际调用仍必须由 Instance 选择，不能据此直接注册某个方法的查询。
#[derive(Default)]
struct QueryCarriers {
    types: HashSet<DefId>,
    callables: HashSet<DefId>,
}
impl QueryCarriers {
    fn add(&mut self, tcx: TyCtxt<'_>, method: DefId) -> Vec<DefId> {
        self.callables.insert(method);
        let mut added = Vec::new();
        if tcx.def_kind(method) == DefKind::AssocFn
            && let Some(implementation) = tcx.impl_of_assoc(method)
        {
            let self_type = tcx
                .type_of(implementation)
                .instantiate_identity()
                .skip_normalization();
            let mut arguments = self_type.walk();
            while let Some(argument) = arguments.next() {
                if let Some(value) = argument.as_type()
                    && let ty::Adt(definition, _) = value.kind()
                    && business_definition(tcx, definition.did())
                {
                    if self.types.insert(definition.did()) {
                        added.push(definition.did());
                    }
                    // Wrapper<PhantomData<T>> 的分派身份是 Wrapper；其泛型内部
                    // 不能独立把 PhantomData、Vec 或另一业务类型变成查询载体。
                    // 引用与标准库基础包装则继续向内寻找实际业务名义类型。
                    arguments.skip_current_subtree();
                }
            }
        }
        added
    }

    fn contains<'tcx>(
        &self,
        tcx: TyCtxt<'tcx>,
        identities: &NominalIdentities<'tcx>,
        value: QueryValue<'tcx>,
    ) -> bool {
        identities
            .identities(tcx, value)
            .iter()
            .any(|id| self.types.contains(id) || self.callables.contains(id))
    }
}

// 有限的名义字段/关联类型图，按定义缓存相邻类型，避免每个调用重复扫描 impl。
// 不物化不断变化的递归泛型实参；此图只用于相关性粗筛。
#[derive(Default)]
struct NominalIdentities<'tcx> {
    edges: RefCell<HashMap<DefId, Vec<Ty<'tcx>>>>,
}
impl<'tcx> NominalIdentities<'tcx> {
    fn identities(&self, tcx: TyCtxt<'tcx>, value: QueryValue<'tcx>) -> HashSet<DefId> {
        let types = match value {
            QueryValue::Service(value) | QueryValue::Callable(value) => vec![value],
            QueryValue::Constant(_, args) => args.types().collect(),
        };
        let mut pending = types;
        let mut identities = HashSet::new();
        let mut fields_seen = HashSet::new();
        let mut aliases_seen = HashSet::new();
        while let Some(value) = pending.pop() {
            for argument in value.walk() {
                let Some(value) = argument.as_type() else {
                    continue;
                };
                match *value.kind() {
                    ty::Adt(definition, _) => {
                        identities.insert(definition.did());
                        // 这里只求有限的名义类型关系。实际实参已由 walk 保留，字段
                        // 使用定义自身的泛型参数，避免 Recursive<Vec<T>> 在粗筛中
                        // 无限扩张。普通函数调用也需要这条关系，例如 drop(Opaque)。
                        if business_definition(tcx, definition.did())
                            && fields_seen.insert(definition.did())
                        {
                            let mut edges = self.edges.borrow_mut();
                            let fields = edges.entry(definition.did()).or_insert_with(|| {
                                let args =
                                    ty::GenericArgs::identity_for_item(tcx, definition.did());
                                definition
                                    .all_fields()
                                    .map(|field| field.ty(tcx, args).skip_normalization())
                                    .collect()
                            });
                            pending.extend(fields.iter().copied());
                        }
                    }
                    ty::Alias(_, alias) => {
                        let id = match alias.kind {
                            ty::Projection { def_id }
                            | ty::Inherent { def_id }
                            | ty::Opaque { def_id }
                            | ty::Free { def_id } => def_id,
                        };
                        if !aliases_seen.insert(id) {
                            continue;
                        }
                        let mut edges = self.edges.borrow_mut();
                        let types = edges.entry(id).or_insert_with(|| {
                            let mut types = Vec::new();
                            // 关联类型的已有 impl/default 只贡献相关性身份。闭合后的
                            // 原生 drop glue 决定实际实现，不能把候选直接变成查询根。
                            if let ty::Projection { def_id } = alias.kind {
                                if tcx.defaultness(def_id).has_value() {
                                    types.push(
                                        tcx.type_of(def_id)
                                            .instantiate_identity()
                                            .skip_normalization(),
                                    );
                                }
                                if let Some(trait_id) = tcx.trait_of_assoc(def_id) {
                                    for implementation in tcx.all_impls(trait_id) {
                                        if let Some(&item) = tcx
                                            .impl_item_implementor_ids(implementation)
                                            .get(&def_id)
                                        {
                                            types.push(
                                                tcx.type_of(item)
                                                    .instantiate_identity()
                                                    .skip_normalization(),
                                            );
                                        }
                                    }
                                }
                            } else {
                                types.push(
                                    tcx.type_of(id).instantiate_identity().skip_normalization(),
                                );
                            }
                            types
                        });
                        pending.extend(types.iter().copied());
                    }
                    ty::FnDef(id, _)
                    | ty::Closure(id, _)
                    | ty::Coroutine(id, _)
                    | ty::CoroutineClosure(id, _) => {
                        identities.insert(id);
                    }
                    _ => {}
                }
            }
        }
        identities
    }
}

fn closed(value: Ty<'_>) -> bool {
    !value.has_non_region_param() && !value.has_infer() && !value.has_escaping_bound_vars()
}

/// 每个编译入口汇总全部已类型检查的 body。非泛型 body 即使没有被调用也贡献查询；
/// 泛型 body 的闭合查询同样直接贡献，带参数的查询则等待实际闭合函数项进行替换。
/// 业务部分沿保存的语义摘要遍历；标准库只补充闭合回调的原生 MIR 转发边。
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
        let mut records = local_summary(tcx, owner);
        // 原生 drop elaboration 后已把隐式析构保存在入口 marker 中。只读取函数类 owner，
        // 并跳过正在据本查询生成的计划入口，避免 MIR 查询递归。
        if !crate::di_plan::is_entry(tcx, owner)
            && business_definition(tcx, owner.to_def_id())
            && matches!(
                tcx.def_kind(owner),
                DefKind::Fn | DefKind::AssocFn | DefKind::Closure | DefKind::SyntheticCoroutineBody
            )
        {
            records.extend(
                external_summary(tcx, owner.to_def_id())
                    .into_iter()
                    .filter(|record| drop_type(tcx, record.value).is_some()),
            );
        }
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
    let mut carrier_users: HashMap<DefId, Vec<DefId>> = HashMap::new();
    let mut carriers = QueryCarriers::default();
    let identities = NominalIdentities::default();
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
            for identity in identities.identities(tcx, record.value) {
                carrier_users.entry(identity).or_default().push(id);
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
            propagation.extend(carrier_users.get(&id).into_iter().flatten().copied());
            // 一个转发层变得相关后，它所属的业务类型又可能使另一层标准转发
            // 相关。类型到 owner 的反向索引与调用边共用工作队列直到固定点，
            // 不只计算一次载体，也不为每层反复扫描全部摘要。
            for identity in carriers.add(tcx, id) {
                propagation.extend(carrier_users.get(&identity).into_iter().flatten().copied());
            }
        }
    }
    let relevant_record = |record: &Record<'tcx>| {
        matches!(record.value, QueryValue::Service(_))
            || carriers.contains(tcx, &identities, record.value)
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
                pending.push_back((record, owner, 0usize, record.span));
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
                    pending.push_back((Record { value, ..record }, owner, 0, record.span));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut seen_roots = HashSet::new();
    let mut roots = Vec::new();
    let mut count = 0usize;
    while let Some((mut record, owner, depth, origin)) = pending.pop_front() {
        count += 1;
        if count > MAX_QUERY_TYPES {
            expansion_error(
                tcx,
                record.span,
                origin,
                "query_instances",
                MAX_QUERY_TYPES,
                "DI 查询泛型实例展开超过有限分析上限，可能存在不断增长的递归泛型调用".into(),
            );
        }
        record.value = record.value.normalize(tcx, record.span, origin)?;
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
        let mut drop_instance = None;
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
                    if matches!(
                        instance.def,
                        ty::InstanceKind::Shim(ty::ShimKind::DropGlue(..))
                    ) {
                        drop_instance = Some(instance);
                    }
                    (instance.def_id(), instance.args)
                }
                Some(_) => continue,
                None => continue,
            }
        };
        if !seen.insert((id, args)) {
            continue;
        }
        if let Some(instance) = drop_instance {
            // rustc 的真实胶水覆盖用户 Drop、聚合字段、Box/Vec/数组等结构。
            // 胶水按 Instance（含实际 T）生成，不能按公共 drop 函数 DefId 缓存。
            for nested in mir_summary(tcx, tcx.instance_mir(instance.def)) {
                if relevant_record(&nested) {
                    pending.push_back((nested, owner, depth + 1, origin));
                }
            }
            continue;
        }
        if standard_definition(tcx, id) {
            summaries
                .entry(id)
                .or_insert_with(|| standard_summary(tcx, id));
        }
        let Some(records) = summaries.get(&id) else {
            continue;
        };
        for &nested in records {
            let value = nested.value.instantiate(tcx, args);
            let nested = Record { value, ..nested };
            if relevant_record(&nested) {
                pending.push_back((nested, owner, depth + 1, origin));
            }
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
