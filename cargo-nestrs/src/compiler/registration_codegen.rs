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

type MirBuilt = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx Steal<mir::Body<'tcx>>;
static ORIGINAL_MIR_BUILT: OnceLock<MirBuilt> = OnceLock::new();

/// 与普通 rustc MIR 构建组合，只追加编译期摘要，绝不改写业务方法的返回或借用。
pub fn provide(providers: &mut rustc_middle::util::Providers) {
    ORIGINAL_MIR_BUILT.get_or_init(|| providers.queries.mir_built);
    providers.queries.mir_built = reflection_mir;
}

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
    struct References<'tcx> {
        tcx: TyCtxt<'tcx>,
    }
    impl References<'_> {
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
pub(crate) enum Kind {
    Provider,
    Binding,
    AutomaticBinding,
}

impl Kind {
    fn from_callback(name: &str) -> Option<Self> {
        match name {
            "__nestrs_reflect_provider" | "__nestrs_reflected_factory" => Some(Self::Provider),
            "__nestrs_reflect_trait_binding" => Some(Self::Binding),
            "__nestrs_reflect_automatic_binding" => Some(Self::AutomaticBinding),
            _ => None,
        }
    }

    pub(crate) fn descriptor(self) -> &'static str {
        match self {
            Self::Provider => "activation::adapter::ActivationAdapter",
            Self::Binding | Self::AutomaticBinding => "activation::adapter::ProjectionAdapter",
        }
    }
}

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
    pub(crate) definition: DefId,
    pub(crate) arguments: ty::GenericArgsRef<'tcx>,
    pub(crate) operands: &'tcx [Spanned<Operand<'tcx>>],
    pub(crate) body: &'tcx mir::Body<'tcx>,
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
                .is_some_and(|name| name.as_str() == "__nestrs_constructor_dependencies")
            {
                validate_constructor_dependencies(tcx, definition, arguments)?;
                pending.push_back(ty::Instance::new_raw(definition, arguments));
            }
            output.push(DescriptorCall {
                definition,
                arguments,
                operands: args.as_ref(),
                body,
            });
        }
    }
    Ok(output)
}

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

fn callback_kind(tcx: TyCtxt<'_>, definition: DefId) -> Option<Kind> {
    let name = tcx.opt_item_name(definition)?;
    // 旧蓝图载体已经由编译器的真实 Ty/Instance 索引取代。保留名称仍不能由业务
    // 伪造以混入反射目录；明确拒绝也能诊断依赖使用了不匹配的旧工具链。
    if matches!(
        name.as_str(),
        "__nestrs_reflect_blueprint" | "__nestrs_reflect_blueprint_path" | "__nestrs_query_root"
    ) {
        tcx.dcx().span_fatal(
            tcx.def_span(definition),
            "Nestrs registration callback names are reserved for authenticated declarations; obsolete blueprint callbacks are unsupported",
        );
    }
    let kind = Kind::from_callback(name.as_str())?;
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
