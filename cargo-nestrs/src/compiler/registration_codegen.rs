//! 编译器拥有的 Nestrs 反射收集：认证目标端 adapter，并保留跨 crate 查询摘要。
//!
//! 这里不再生成注册快照或调用可变 registry。完整声明只存在于可信反射标记中；
//! 宏回调返回 core 能直接执行的 ActivationAdapter / ProjectionAdapter，最终计划
//! 由 di_plan 写入唯一反射入口。收集器只检查签名和编码 MIR，不执行用户构造。

extern crate rustc_abi;
extern crate rustc_data_structures;

use rustc_abi::ExternAbi;
use rustc_data_structures::steal::Steal;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, DefIndex, LOCAL_CRATE, LocalDefId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_middle::mir::{self, Operand, TerminatorKind};
use rustc_middle::ty::{self, Ty, TyCtxt};
use rustc_span::{Span, Spanned};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::OnceLock;

/// 原生 MIR 查询签名，覆盖构建与析构展开两个阶段。
type MirBuilt = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<mir::Body<'tcx>>;

/// 保存原构建 MIR 查询，摘要写入前先获取其正常结果。
static ORIGINAL_MIR_BUILT: OnceLock<MirBuilt> = OnceLock::new();

/// 保存原析构展开查询，保证 Drop 摘要沿用 rustc 真实规则。
static ORIGINAL_MIR_DROPS: OnceLock<MirBuilt> = OnceLock::new();

/// 与普通 rustc MIR 构建组合，只追加编译期摘要，绝不改写业务方法的返回或借用。
pub fn provide(providers: &mut rustc_middle::util::Providers) {
    ORIGINAL_MIR_BUILT.get_or_init(|| providers.queries.mir_built);
    providers.queries.mir_built = reflection_mir;
    ORIGINAL_MIR_DROPS.get_or_init(|| providers.queries.mir_drops_elaborated_and_const_checked);
    providers.queries.mir_drops_elaborated_and_const_checked = reflection_drop_mir;
}

/// 在原生构建 MIR 中保存查询摘要，保留后续标准 rustc 流程。
fn reflection_mir(tcx: TyCtxt<'_>, definition: LocalDefId) -> &Steal<mir::Body<'_>> {
    let original = ORIGINAL_MIR_BUILT
        .get()
        .expect("Nestrs reflection MIR provider was installed")(tcx, definition);
    if tcx.crate_name(LOCAL_CRATE).as_str() == "nestrs_core" {
        return original;
    }
    let mut body = original.steal();
    crate::query_roots::preserve_summary(tcx, definition, &mut body);
    tcx.alloc_steal_mir(body)
}

// 原生 move 分析与 drop elaboration 决定哪些值真正需要析构；在此之前记录 Drop
// 会把已移入 forget/ManuallyDrop 的临时值也加入图。此查询仍先于优化和常量分支消除。
/// 在原生析构展开之后保存真实 Drop 摘要。
fn reflection_drop_mir(tcx: TyCtxt<'_>, definition: LocalDefId) -> &Steal<mir::Body<'_>> {
    let original =
        ORIGINAL_MIR_DROPS
            .get()
            .expect("Nestrs drop summary MIR provider was installed")(tcx, definition);
    if tcx.crate_name(LOCAL_CRATE).as_str() == "nestrs_core" && !tcx.sess.opts.test {
        return original;
    }
    let mut body = original.steal();
    crate::query_roots::preserve_drop_summary(tcx, &mut body);
    tcx.alloc_steal_mir(body)
}

/// 反射身份属于工具生成作用域，而不是 core 内的某个空标记函数。
/// 名称、作用域、生成来源与类型形状必须同时成立；普通业务同名项没有反射权限。
pub(crate) fn reflect_item(tcx: TyCtxt<'_>, definition: DefId, name: &str) -> bool {
    crate::reflection::reflect_item(tcx, definition, name)
}

/// 配置归最终入口所属 package 所有。CARGO_MANIFEST_* 由 Cargo 为每个编译单元设置，
/// 因此上游 rlib 的 metadata 不会把自己的启动策略传播到宿主 binary/test。
///
/// 使用 SourceMap 而非直接 fs::read，令 Cargo.toml 同时进入 rustc dep-info 和现有
/// SnapshotLoader/CheckedLoader：仅修改 [nestrs-cli] 也必须重编译，两个编译阶段之间
/// 改动文件则拒绝继续。编译后的程序只含配置值，运行时无需部署或读取 Cargo.toml。
pub(crate) fn startup_options(
    session: &rustc_session::Session,
) -> cargo_nestrs::project_config::DiConfig {
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
    /// 检查源码对保留计划入口的直接和别名引用。
    struct References<'tcx> {
        /// 用于确认定义身份及报告位置的当前会话。
        tcx: TyCtxt<'tcx>,
    }

    impl References<'_> {
        /// 拒绝业务代码直接引用编译器保留的计划入口。
        fn check(&self, definition: DefId, span: Span) {
            if let Some(local) = definition.as_local()
                && crate::di_plan::is_entry(self.tcx, local)
            {
                self.tcx.dcx().span_fatal(
                    span,
                    "the compiler-owned Nestrs reflection entry cannot be referenced by Rust source",
                );
            }
        }
    }

    impl<'tcx> Visitor<'tcx> for References<'tcx> {
        /// 包含嵌套定义的 HIR 引用审计范围。
        type NestedFilter = rustc_middle::hir::nested_filter::All;

        /// 提供上下文以遍历嵌套 HIR 定义。
        fn maybe_tcx(&mut self) -> TyCtxt<'tcx> {
            self.tcx
        }

        /// 检查已解析路径是否引用保留入口。
        fn visit_path(&mut self, path: &rustc_hir::Path<'tcx>, _: rustc_hir::HirId) {
            if let Some(definition) = path.res.opt_def_id() {
                self.check(definition, path.span);
            }
            intravisit::walk_path(self, path);
        }

        /// 检查 import/reexport 的真实目标，避免通过别名绕过入口检查。
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

        /// 补查类型检查后解析的函数项与关联表达式。
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

/// 已认证反射回调的用途及预期 adapter 形态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    /// 提供服务构造能力的回调。
    Provider,

    /// 业务显式绑定的投影回调。
    Binding,

    /// 仅在存在实际接口需求时启用的自动投影回调。
    AutomaticBinding,
}

impl Kind {
    /// 按保留拼写筛选候选用途，调用者仍需核对真实生成来源。
    fn from_callback(name: &str) -> Option<Self> {
        match name {
            "__nestrs_reflect_provider" | "__nestrs_reflected_factory" => Some(Self::Provider),
            "__nestrs_reflect_trait_binding" => Some(Self::Binding),
            "__nestrs_reflect_automatic_binding" => Some(Self::AutomaticBinding),
            _ => None,
        }
    }

    /// 返回该回调用途要求的 core adapter 定义路径。
    pub(crate) fn descriptor(self) -> &'static str {
        match self {
            Self::Provider => crate::protocol::ACTIVATION_ADAPTER,
            Self::Binding | Self::AutomaticBinding => crate::protocol::PROJECTION_ADAPTER,
        }
    }
}

/// 从真实 DefPath 构造内部路径，避免诊断打印器的 facade 别名影响身份检查。
pub(crate) fn definition_path(tcx: TyCtxt<'_>, definition: DefId) -> String {
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

/// 汇总本地与上游可用的已认证回调，按稳定身份排序并去重。
pub(crate) fn collect_callbacks(tcx: TyCtxt<'_>) -> Vec<(Kind, DefId)> {
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

/// 单个声明调用及其原始 MIR；key 字面量中的局部变量索引只属于这个 body。
pub(crate) struct DescriptorCall<'tcx> {
    /// 已认证的声明 marker 定义。
    pub(crate) definition: DefId,

    /// 代入当前闭合实例后的真实泛型实参。
    pub(crate) arguments: ty::GenericArgsRef<'tcx>,

    /// 调用的原始 MIR 操作数，索引属于下方 body。
    pub(crate) operands: &'tcx [Spanned<Operand<'tcx>>],

    /// 操作数所属的 MIR，用于追踪 key 与字面量。
    pub(crate) body: &'tcx mir::Body<'tcx>,

    /// marker 的调用位置保留原字段/参数 token；不能用描述函数位置替代。
    pub(crate) span: Span,
}

/// 读取已经选中的类型化声明，按需进入工具生成的 constructor 依赖描述 helper。
///
/// helper 可以位于上游 crate 或泛型 impl 中。每一步都以当前 Instance 实参替换调用
/// 实参，保留真实类型身份；遍历从不执行函数，也不进入用户 constructor/factory。
/// 名称只是入口筛选，必须同时验证宏来源、inherent 方法形态和准确描述返回类型。
pub(crate) fn descriptor_calls<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: ty::Instance<'tcx>,
) -> Result<Vec<DescriptorCall<'tcx>>, String> {
    let mut pending = VecDeque::from([instance]);
    let mut visited = HashSet::new();
    let mut output = Vec::new();
    while let Some(instance) = pending.pop_front() {
        if !visited.insert(instance) {
            continue;
        }
        if !tcx.is_mir_available(instance.def_id()) {
            return Err(format!(
                "DI 描述 {} 缺少编码 MIR；请使用匹配的 cargo nestrs 重编译依赖",
                tcx.def_path_str(instance.def_id())
            ));
        }
        let body = tcx.optimized_mir(instance.def_id());
        for block in body.basic_blocks.iter() {
            let TerminatorKind::Call { func, args, .. } = &block.terminator().kind else {
                continue;
            };
            let ty::FnDef(definition, arguments) = *func.ty(&body.local_decls, tcx).kind() else {
                continue;
            };
            let arguments = ty::EarlyBinder::bind(tcx, arguments)
                .instantiate(tcx, instance.args)
                .skip_normalization();
            if tcx
                .opt_item_name(definition)
                .is_some_and(|name| name.as_str() == crate::protocol::constructor::DEPENDENCIES)
            {
                validate_constructor_dependencies(tcx, definition, arguments)?;
                pending.push_back(ty::Instance::new_raw(definition, arguments));
            }
            output.push(DescriptorCall {
                definition,
                arguments,
                operands: args.as_ref(),
                body,
                span: block.terminator().source_info.span,
            });
        }
    }
    Ok(output)
}

/// 核对构造依赖 helper 的生成来源、inherent 身份及 `Vec<InputAdapter>` 签名。
fn validate_constructor_dependencies<'tcx>(
    tcx: TyCtxt<'tcx>,
    definition: DefId,
    arguments: ty::GenericArgsRef<'tcx>,
) -> Result<(), String> {
    let invalid = || {
        format!(
            "constructor 依赖描述 helper {} 必须来自真实 Nestrs 生成代码，且为无参数的 inherent 方法并返回 Vec<InputAdapter>",
            tcx.def_path_str(definition)
        )
    };
    if tcx.def_kind(definition) != DefKind::AssocFn
        || !crate::internal_access::trusted_definition(tcx, definition)
    {
        return Err(invalid());
    }
    let parent = tcx.parent(definition);
    if tcx.def_kind(parent) != (DefKind::Impl { of_trait: false })
        || tcx.generics_of(definition).count() != tcx.generics_of(parent).count()
    {
        return Err(invalid());
    }
    let signature = tcx
        .fn_sig(definition)
        .instantiate(tcx, arguments)
        .skip_normalization()
        .skip_binder();
    let ty::Adt(vector, elements) = signature.output().kind() else {
        return Err(invalid());
    };
    if tcx.crate_name(vector.did().krate).as_str() != "alloc"
        || definition_path(tcx, vector.did()) != "vec::Vec"
    {
        return Err(invalid());
    }
    let valid_element = matches!(elements.type_at(0).kind(), ty::Adt(element, _) if
        tcx.crate_name(element.did().krate).as_str() == "nestrs_core"
        && definition_path(tcx, element.did()) == "activation::adapter::InputAdapter");
    if !signature.inputs().is_empty()
        || signature.abi() != ExternAbi::Rust
        || !signature.safety().is_safe()
        || signature.c_variadic()
        || !valid_element
    {
        return Err(invalid());
    }
    Ok(())
}

/// 认证候选回调来源和形态，拒绝旧工具协议而不误拒普通业务同名项。
fn callback_kind(tcx: TyCtxt<'_>, definition: DefId) -> Option<Kind> {
    let name = tcx.opt_item_name(definition)?;
    let obsolete = matches!(
        name.as_str(),
        "__nestrs_reflect_blueprint" | "__nestrs_reflect_blueprint_path" | "__nestrs_query_root"
    );
    let kind = Kind::from_callback(name.as_str());
    if !obsolete && kind.is_none() {
        return None;
    }
    // 名称仅筛选已经认证的生成项。普通业务可以使用同样的函数/成员名，
    // 但不会因此贡献注册；其余私有 ABI 访问仍由独立的来源审计拒绝。
    if !crate::internal_access::trusted_definition(tcx, definition) {
        return None;
    }
    // 旧蓝图载体已经由真实 Ty/Instance 索引取代。只对认证的旧工具产物诊断
    // 协议不匹配，不能把普通业务的同名函数误当成旧工具产物。
    if obsolete {
        tcx.dcx().span_fatal(
            tcx.def_span(definition),
            "Nestrs registration callback names are reserved for authenticated declarations; obsolete blueprint callbacks are unsupported",
        );
    }
    let kind = kind?;
    if tcx.def_kind(definition) != DefKind::Fn || tcx.is_foreign_item(definition) {
        tcx.dcx().span_fatal(
            tcx.def_span(definition),
            "Nestrs registration callback names are reserved for authenticated declarations",
        );
    }
    Some(kind)
}

/// 检查闭合回调的完整签名与 adapter 类型，返回可供计划使用的输出类型。
pub(crate) fn callback_descriptor<'tcx>(
    tcx: TyCtxt<'tcx>,
    kind: Kind,
    callback: DefId,
) -> Ty<'tcx> {
    if tcx.generics_of(callback).count() != 0 {
        tcx.dcx()
            .fatal("Nestrs reflection callbacks must have closed signatures");
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
            "incompatible Nestrs reflection callback ABI for {}",
            tcx.def_path_str(callback),
        ));
    }
    output
}
