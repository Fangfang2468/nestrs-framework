//! Compiler-owned discovery for the automatic binding experiment.
//!
//! Hidden marker functions describe DI intent. Types and candidate
//! applicability come from rustc; source text is only retained for the final
//! source overlay, never searched for services or impls.

extern crate rustc_ast;
extern crate rustc_infer;
extern crate rustc_span;
extern crate rustc_trait_selection;

use crate::autobind_codegen::{BindingSpec, SourceInsertion};
use crate::protocol::{self, Marker as ReflectionMarker};
use crate::registration_codegen::reflect_item;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, DefIndex, LocalDefId, LocalModDefId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_infer::infer::TyCtxtInferExt;
use rustc_middle::mir;
use rustc_middle::ty::{self, Ty, TyCtxt, TypeVisitableExt, Upcast as _};
use rustc_span::{FileName, Span};
use rustc_trait_selection::{
    infer::InferCtxtExt,
    traits::{self, Obligation, ObligationCause, ObligationCtxt},
};
use std::collections::{HashMap, HashSet, VecDeque};

pub struct Analysis {
    pub insertions: Vec<SourceInsertion>,
    pub providers: usize,
    pub requests: usize,
    pub generated_bindings: usize,
    pub explicit_bindings: usize,
    pub automatic_bindings: usize,
    pub blueprints: usize,
    pub generated_blueprints: usize,
    /// Diagnostic records only. Selection and deduplication use rustc Ty identity.
    pub automatic_projections: Vec<(String, String)>,
    pub explicit_projections: Vec<(String, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkerKind {
    Provider,
    Request,
    Binding,
    AutomaticBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum CompilerKey {
    Default,
    Named(String),
    Indexed(u128),
}

#[derive(Clone)]
struct Marker<'tcx> {
    kind: MarkerKind,
    types: Vec<Ty<'tcx>>,
    owner: LocalDefId,
    span: Span,
    key: Option<CompilerKey>,
    passive: bool,
}

struct MarkerVisitor<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    typeck: &'tcx ty::TypeckResults<'tcx>,
    output: &'a mut Vec<Marker<'tcx>>,
    errors: &'a mut Vec<String>,
}

impl<'tcx> Visitor<'tcx> for MarkerVisitor<'_, 'tcx> {
    fn visit_expr(&mut self, expr: &'tcx rustc_hir::Expr<'tcx>) {
        if let rustc_hir::ExprKind::Call(function, arguments) = expr.kind
            && let ty::FnDef(def_id, args) = *self.typeck.expr_ty(function).kind()
        {
            let kind = marker_kind(self.tcx, def_id);
            if let Some(kind) = kind {
                let key = if kind == MarkerKind::Provider {
                    match self.provider_key(arguments) {
                        Ok(key) => Some(key),
                        Err(error) => {
                            self.errors.push(error);
                            return;
                        }
                    }
                } else {
                    None
                };
                self.output.push(Marker {
                    kind,
                    types: args.types().collect(),
                    owner: self.owner,
                    span: expr.span,
                    key,
                    passive: false,
                });
            }
        }
        intravisit::walk_expr(self, expr);
    }
}

impl MarkerVisitor<'_, '_> {
    fn provider_key(&self, arguments: &[rustc_hir::Expr<'_>]) -> Result<CompilerKey, String> {
        let [argument] = arguments else {
            return Err(
                "incompatible compiler_provider marker ABI: expected one CompilerKey".into(),
            );
        };
        let (definition, literal) = match argument.kind {
            rustc_hir::ExprKind::Path(ref path) => {
                let rustc_hir::def::Res::Def(_, definition) =
                    self.typeck.qpath_res(path, argument.hir_id)
                else {
                    return Err("CompilerKey marker does not resolve to a key variant".into());
                };
                let definition = if matches!(self.tcx.def_kind(definition), DefKind::Ctor(..)) {
                    self.tcx.parent(definition)
                } else {
                    definition
                };
                (definition, None)
            }
            rustc_hir::ExprKind::Call(function, [value]) => {
                let ty::FnDef(constructor, _) = *self.typeck.expr_ty(function).kind() else {
                    return Err("CompilerKey marker is not an enum constructor".into());
                };
                let rustc_hir::ExprKind::Lit(literal) = &value.kind else {
                    return Err("CompilerKey marker requires a declaration literal".into());
                };
                (self.tcx.parent(constructor), Some(literal.node))
            }
            _ => return Err("unsupported CompilerKey marker expression".into()),
        };
        if !reflect_item(
            self.tcx,
            self.tcx.parent(definition),
            protocol::COMPILER_KEY,
        ) {
            return Err(
                "CompilerKey marker variant belongs to an unauthenticated declaration".into(),
            );
        }
        match (self.tcx.item_name(definition).as_str(), literal) {
            ("Default", None) => Ok(CompilerKey::Default),
            ("Named", Some(rustc_ast::LitKind::Str(name, _))) => {
                Ok(CompilerKey::Named(name.to_string()))
            }
            ("Indexed", Some(rustc_ast::LitKind::Int(value, _))) => {
                Ok(CompilerKey::Indexed(value.0))
            }
            _ => Err("incompatible CompilerKey marker variant or literal".into()),
        }
    }
}

fn closed(ty: Ty<'_>) -> bool {
    !ty.has_non_region_param() && !ty.has_infer() && !ty.has_escaping_bound_vars()
}

fn normalized<'tcx>(
    tcx: TyCtxt<'tcx>,
    owner: LocalDefId,
    ty: Ty<'tcx>,
) -> Result<Ty<'tcx>, String> {
    crate::query_roots::validate_type_complexity(tcx, ty)?;
    tcx.try_normalize_erasing_regions(
        if tcx.def_kind(owner) == DefKind::Mod {
            ty::TypingEnv::fully_monomorphized()
        } else {
            ty::TypingEnv::post_analysis(tcx, owner)
        },
        ty::Unnormalized::new_wip(ty),
    )
    .map_err(|error| format!("cannot normalize DI type {ty}: {error:?}"))
}

/// 每份泛型声明拥有自己的私有反射 trait。这里只从实际服务所属 crate 的
/// 已认证声明中求解闭合实现，不需要 core 提供全局 ProviderDefinition 协议。
pub(crate) fn provider_definition<'tcx>(
    tcx: TyCtxt<'tcx>,
    service: Ty<'tcx>,
) -> Option<(DefId, DefId)> {
    let ty::Adt(service_definition, _) = service.kind() else {
        return None;
    };
    let infcx = tcx
        .infer_ctxt()
        .ignoring_regions()
        .build(ty::TypingMode::non_body_analysis());
    let mut selected = None;
    for &trait_id in tcx.traits(service_definition.did().krate) {
        if !reflect_item(tcx, trait_id, protocol::PROVIDER_DEFINITION)
            || !infcx
                .type_implements_trait(trait_id, [service], ty::ParamEnv::empty())
                .must_apply_modulo_regions()
        {
            continue;
        }
        let Some(method) = tcx
            .associated_items(trait_id)
            .in_definition_order()
            .find(|item| item.name().as_str() == "provider")
        else {
            continue;
        };
        if selected.replace((trait_id, method.def_id)).is_some() {
            tcx.dcx()
                .fatal(format!("服务 {service} 匹配多个工具生成的构造蓝图"));
        }
    }
    selected
}

/// 返回真实类型经 trait 求解得到的构造入口。私有类型只存在于 rustc Ty 中，
/// 不生成跨 crate 源码路径，也不提升业务类型的可见性。
pub(crate) fn provider_blueprint<'tcx>(
    tcx: TyCtxt<'tcx>,
    service: Ty<'tcx>,
) -> Result<Option<ty::Instance<'tcx>>, String> {
    let Some((_, method)) = provider_definition(tcx, service) else {
        return Ok(None);
    };
    ty::Instance::try_resolve(
        tcx,
        ty::TypingEnv::fully_monomorphized(),
        method,
        tcx.mk_args(&[service.into()]),
    )
    .map_err(|_| format!("无法解析闭合服务蓝图 {service}"))
}

/// MIR 调用局部 trait 的普通泛型 helper；不能把 impl 方法伪装成可直接调用的 FnDef。
pub(crate) fn provider_callback<'tcx>(
    tcx: TyCtxt<'tcx>,
    service: Ty<'tcx>,
) -> Result<ty::Instance<'tcx>, String> {
    let (trait_id, _) = provider_definition(tcx, service)
        .ok_or_else(|| format!("服务 {service} 缺少工具生成的构造蓝图"))?;
    let module = tcx.parent(trait_id);
    // module_children 是 rustc 的外部 metadata query，不能传入本地 DefId。
    // 本地只遍历已有 HIR body，避免在 MIR 构造中要求已完成全 crate 分析。
    let helper = if module.is_local() {
        tcx.hir_body_owners()
            .map(LocalDefId::to_def_id)
            .find(|&definition| {
                tcx.parent(definition) == module
                    && reflect_item(tcx, definition, protocol::PROVIDER_HELPER)
            })
    } else {
        tcx.module_children(module)
            .iter()
            .filter_map(|child| child.res.opt_def_id())
            .find(|&definition| reflect_item(tcx, definition, protocol::PROVIDER_HELPER))
    }
    .ok_or_else(|| format!("服务 {service} 缺少同声明的 typed 构造 helper"))?;
    Ok(ty::Instance::new_raw(
        helper,
        tcx.mk_args(&[service.into()]),
    ))
}

fn marker_kind(tcx: TyCtxt<'_>, definition: DefId) -> Option<MarkerKind> {
    [
        (ReflectionMarker::Provider.name(), MarkerKind::Provider),
        (ReflectionMarker::Dependency.name(), MarkerKind::Request),
        (ReflectionMarker::Binding.name(), MarkerKind::Binding),
        (
            ReflectionMarker::AutomaticBinding.name(),
            MarkerKind::AutomaticBinding,
        ),
    ]
    .into_iter()
    .find_map(|(name, kind)| reflect_item(tcx, definition, name).then_some(kind))
}

fn closed_provider_markers<'tcx>(
    tcx: TyCtxt<'tcx>,
    method: DefId,
    service: Ty<'tcx>,
    owner: LocalDefId,
    span: Span,
) -> Result<Vec<Marker<'tcx>>, String> {
    let instance = ty::Instance::try_resolve(
        tcx,
        ty::TypingEnv::fully_monomorphized(),
        method,
        tcx.mk_args(&[service.into()]),
    )
    .map_err(|_| format!("rustc could not resolve provider blueprint for {service}"))?
    .ok_or_else(|| format!("provider blueprint for {service} is not fully resolved"))?;
    // 本地与跨 crate 蓝图使用同一份 MIR 描述遍历。constructor 的输入位于独立
    // associated helper 中，不能只取 ProviderDefinition::provider 自身的 HIR marker。
    // 本地仍用 provider 的词法模块作为投影插入上下文；上游则使用请求方上下文。
    let owner = instance.def_id().as_local().unwrap_or(owner);
    external_blueprint_markers(tcx, instance, owner, span, true)
}

/// Combine this crate's declarations with typed registration metadata from
/// dependencies processed by the framework wrapper. Insertions remain local;
/// private upstream projections come from the producer's capability catalog.
pub fn analyze<'tcx>(tcx: TyCtxt<'tcx>) -> Result<Analysis, String> {
    let runtimes: Vec<_> = tcx
        .crates(())
        .iter()
        .filter(|&&krate| tcx.crate_name(krate).as_str() == "nestrs_core")
        .collect();
    if runtimes.len() > 1 {
        return Err("multiple nestrs-core crate identities are linked; distributed DI registrations require a single compatible nestrs-core version".into());
    }
    let mut markers = Vec::new();
    let mut errors = Vec::new();
    for owner in tcx.hir_body_owners() {
        let body = tcx.hir_body_owned_by(owner);
        MarkerVisitor {
            tcx,
            owner,
            typeck: tcx.typeck(owner),
            output: &mut markers,
            errors: &mut errors,
        }
        .visit_body(body);
    }
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    markers.extend(external_registrations(tcx)?);
    markers.extend(
        crate::query_roots::collect(tcx)?
            .into_iter()
            .map(|root| Marker {
                kind: MarkerKind::Request,
                types: vec![root.service],
                owner: tcx.parent_module_from_def_id(root.owner).to_local_def_id(),
                span: root.span,
                key: None,
                passive: false,
            }),
    );

    let mut pending = VecDeque::new();
    let mut explicit_providers = HashSet::new();
    for marker in markers {
        // A closed-looking field inside an unused generic blueprint is not a
        // global DI request. Its markers become live only when that blueprint
        // is instantiated through a known closed root or dependency.
        if (tcx.def_kind(marker.owner) == DefKind::Mod
            || tcx.generics_of(marker.owner).count() == 0)
            && marker.types.iter().copied().all(closed)
        {
            if marker.kind == MarkerKind::Provider {
                explicit_providers.insert((
                    normalized(tcx, marker.owner, marker.types[0])?,
                    marker
                        .key
                        .clone()
                        .expect("provider marker always has a key"),
                ));
            }
            pending.push_back(marker.clone());
        }
    }

    let infcx = tcx
        .infer_ctxt()
        .ignoring_regions()
        .build(ty::TypingMode::non_body_analysis());
    let param_env = ty::ParamEnv::empty();
    let mut providers: HashMap<Ty<'_>, Marker<'_>> = HashMap::new();
    let mut capability_candidates: HashMap<Ty<'_>, Marker<'_>> = HashMap::new();
    let mut requests = HashSet::new();
    let mut bindings = HashSet::new();
    let mut automatic_bindings = HashSet::new();
    let mut needed_blueprints = HashMap::new();
    let mut expanded = HashSet::new();
    let mut passive_expanded = HashSet::new();
    let unsize_trait = tcx
        .lang_items()
        .unsize_trait()
        .ok_or_else(|| "rustc Unsize lang item is unavailable".to_string())?;
    // An explicitly closed impl is another finite source of a closed generic
    // type. It is a provider candidate only if rustc proves ProviderDefinition
    // and a currently requested interface can actually receive that type.
    let mut closed_impls = Vec::new();
    for owner in tcx.iter_local_def_id() {
        if tcx.def_kind(owner) != (DefKind::Impl { of_trait: true }) {
            continue;
        }
        let concrete = tcx
            .type_of(owner)
            .instantiate_identity()
            .skip_normalization();
        if closed(concrete) {
            closed_impls.push(Marker {
                kind: MarkerKind::Request,
                types: vec![normalized(tcx, owner, concrete)?],
                owner,
                span: tcx.def_span(owner),
                key: None,
                passive: true,
            });
        }
    }
    pending.extend(closed_impls.iter().cloned());

    let mut all_types = HashSet::new();
    let mut method_query_roots = HashSet::new();
    let mut method_provider_count = 0;
    loop {
        if pending.is_empty() && method_provider_count != providers.len() {
            method_provider_count = providers.len();
            let known: Vec<_> = providers.keys().copied().collect();
            for root in crate::query_roots::collect_with_providers(tcx, &known)? {
                if method_query_roots.insert(root.service) {
                    pending.push_back(Marker {
                        kind: MarkerKind::Request,
                        types: vec![root.service],
                        owner: tcx.parent_module_from_def_id(root.owner).to_local_def_id(),
                        span: root.span,
                        key: None,
                        passive: false,
                    });
                }
            }
        }
        if pending.is_empty() {
            for (&concrete, marker) in &capability_candidates {
                if !expanded.contains(&concrete)
                    && requests.iter().any(|interface| {
                        infcx
                            .type_implements_trait(unsize_trait, [concrete, *interface], param_env)
                            .must_apply_modulo_regions()
                    })
                {
                    let mut marker = marker.clone();
                    marker.kind = MarkerKind::Request;
                    marker.passive = false;
                    pending.push_back(marker);
                }
            }
        }
        let Some(mut marker) = pending.pop_front() else {
            break;
        };
        for ty in &mut marker.types {
            *ty = normalized(tcx, marker.owner, *ty)?;
            if all_types.insert(*ty) && all_types.len() > crate::query_roots::MAX_QUERY_TYPES {
                return Err(
                    "DI 查询/Provider 闭合类型超过 100000 个，可能存在不断增长的递归泛型声明"
                        .into(),
                );
            }
        }
        if !marker.types.iter().copied().all(closed) {
            return Err("DI marker remained generic after closed-type expansion".into());
        }
        match marker.kind {
            MarkerKind::Provider => {
                capability_candidates
                    .entry(marker.types[0])
                    .or_insert_with(|| marker.clone());
                if !marker.passive {
                    providers.entry(marker.types[0]).or_insert(marker);
                }
            }
            MarkerKind::AutomaticBinding => {
                automatic_bindings.insert((marker.types[0], marker.types[1]));
                marker.kind = MarkerKind::Request;
                marker.types.truncate(1);
                marker.passive = true;
                pending.push_back(marker);
            }
            MarkerKind::Binding => {
                bindings.insert((marker.types[0], marker.types[1]));
                // Binding callbacks can materialize a generic even when its
                // interface is never queried. The concrete blueprint therefore
                // contributes dependency demands, but its interface does not.
                marker.kind = MarkerKind::Request;
                marker.types.truncate(1);
                pending.push_back(marker);
            }
            MarkerKind::Request => {
                let requested = marker.types[0];
                if matches!(requested.kind(), ty::Dynamic(..)) {
                    if !marker.passive {
                        requests.insert(requested);
                    }
                    continue;
                }
                if !(if marker.passive {
                    passive_expanded.insert(requested)
                } else {
                    expanded.insert(requested)
                }) {
                    continue;
                }
                let Some((_, method_id)) = provider_definition(tcx, requested) else {
                    continue;
                };
                let mut instantiated_markers =
                    closed_provider_markers(tcx, method_id, requested, marker.owner, marker.span)?;
                for nested in &mut instantiated_markers {
                    nested.passive = marker.passive;
                }
                let blueprint = instantiated_markers
                    .iter()
                    .find(|marker| marker.kind == MarkerKind::Provider)
                    .ok_or_else(|| {
                        format!("provider blueprint {requested} has no provider identity marker")
                    })?;
                let blueprint_key = blueprint
                    .key
                    .clone()
                    .expect("provider marker always has a key");
                if explicit_providers.contains(&(requested, blueprint_key)) {
                    // The graph compiler will select the exact explicit
                    // provider and never visit this fallback's dependencies.
                    continue;
                }
                needed_blueprints
                    .entry(requested)
                    .or_insert_with(|| marker.clone());
                pending.extend(instantiated_markers);
            }
        }
    }

    let mut insertions = Vec::new();
    let mut generated = 0;
    let type_source = crate::type_source::SourceTypes::new(tcx);
    // 有限闭合蓝图直接随真实 Ty/Instance 进入最终计划；无需生成公共锚点或
    // DependencyPath 回调来重新描述相同依赖，私有深链同样按 MIR 迭代展开。
    let blueprints = needed_blueprints.len();
    let mut candidates: Vec<_> = capability_candidates.iter().collect();
    candidates.sort_by_key(|(ty, _)| type_source.render(**ty));
    let mut interfaces: Vec<_> = requests.iter().copied().collect();
    interfaces.sort_by_key(|ty| type_source.render(*ty));
    let potential_impls = capability_impls(tcx);
    let mut generated_projections = Vec::new();
    for (concrete, marker) in candidates {
        let mut candidate_interfaces: HashSet<_> = interfaces.iter().copied().collect();
        candidate_interfaces.extend(declared_interfaces(tcx, *concrete, &potential_impls));
        let mut candidate_interfaces: Vec<_> = candidate_interfaces.into_iter().collect();
        candidate_interfaces.sort_by_key(|ty| type_source.render(*ty));
        for interface in &candidate_interfaces {
            if bindings.contains(&(*concrete, *interface))
                || automatic_bindings.contains(&(*concrete, *interface))
            {
                continue;
            }
            if !infcx
                .type_implements_trait(unsize_trait, [*concrete, *interface], param_env)
                .must_apply_modulo_regions()
            {
                continue;
            }
            let source = tcx
                .sess
                .source_map()
                .lookup_char_pos(marker.span.source_callsite().lo());
            let (Some(concrete_source), Some(interface_source)) = (
                type_source.render_if_nameable(*concrete),
                type_source.render_if_nameable(*interface),
            ) else {
                // 被动能力目录可能包含仅从传递 metadata 看见的 blanket impl。
                // 它不是业务需求，不能为它生成当前 crate 无法命名的源码路径。
                // 已明确请求的接口仍报错，不能静默丢掉真实业务依赖。
                if requests.contains(interface) {
                    return Err(format!(
                        "cannot name automatic binding from {concrete} to {interface}: the generated type path requires a direct extern crate or an accessible reexport; private upstream services require a producer capability"
                    ));
                }
                continue;
            };
            let binding = BindingSpec {
                concrete: concrete_source,
                interface: interface_source,
                source_file: source.file.name.prefer_local_unconditionally().to_string(),
                source_line: source.line as u32,
                source_column: source.col.0 as u32 + 1,
            };
            let insertion = insertion_for(tcx, *concrete, *interface, marker.owner, binding);
            match insertion {
                Ok(insertion) => insertions.push(insertion),
                Err(error) if requests.contains(interface) => return Err(error),
                Err(_) => continue,
            }
            generated += 1;
            generated_projections.push((*concrete, *interface));
        }
    }
    let projection_names = |pairs: Vec<(Ty<'tcx>, Ty<'tcx>)>| {
        let mut names: Vec<_> = pairs
            .into_iter()
            .map(|(concrete, interface)| {
                (
                    rustc_const_eval::util::type_name(tcx, concrete),
                    rustc_const_eval::util::type_name(tcx, interface),
                )
            })
            .collect();
        names.sort();
        names
    };
    let automatic_projections = projection_names(
        automatic_bindings
            .iter()
            .copied()
            .chain(generated_projections)
            .collect(),
    );
    let explicit_projections = projection_names(bindings.iter().copied().collect());
    Ok(Analysis {
        insertions,
        providers: providers.len(),
        requests: requests.len(),
        generated_bindings: generated,
        explicit_bindings: bindings.len(),
        automatic_bindings: automatic_bindings.len(),
        blueprints,
        generated_blueprints: 0,
        automatic_projections,
        explicit_projections,
    })
}

/// Import only actual nongeneric registration callbacks. Template provider
/// methods are read separately after a closed Instance is selected, so a
/// dormant generic declaration cannot create an active dependency request.
fn external_registrations<'tcx>(tcx: TyCtxt<'tcx>) -> Result<Vec<Marker<'tcx>>, String> {
    let mut markers = Vec::new();
    for &krate in tcx.crates(()) {
        for index in 0..tcx.num_extern_def_ids(krate) {
            let definition = DefId {
                krate,
                index: DefIndex::from_usize(index),
            };
            // Metadata tables contain holes: inspect MIR availability first.
            if !tcx.is_mir_available(definition) {
                continue;
            }
            let Some(name) = tcx.opt_item_name(definition) else {
                continue;
            };
            if !matches!(
                name.as_str(),
                "__nestrs_reflect_provider"
                    | "__nestrs_reflected_factory"
                    | "__nestrs_reflect_trait_binding"
                    | "__nestrs_reflect_automatic_binding"
            ) {
                continue;
            }
            if tcx.generics_of(definition).count() != 0 {
                return Err(format!(
                    "upstream registration callback {} unexpectedly captures generic arguments",
                    tcx.def_path_str(definition)
                ));
            }
            let instance = ty::Instance::mono(tcx, definition);
            markers.extend(external_blueprint_markers(
                tcx,
                instance,
                LocalModDefId::CRATE_DEF_ID.to_local_def_id(),
                tcx.def_span(definition),
                false,
            )?);
        }
    }
    Ok(markers)
}

/// Closed generic blueprints encode their MIR in the dependency's metadata.
/// Read only type-bearing marker calls from that body: no constructor is run,
/// source is not inspected, and types are substituted by rustc's Instance args.
/// A local requesting owner supplies a legal candidate insertion context; rustc
/// checks accessibility of external concrete/interface types in the second pass.
fn external_blueprint_markers<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: ty::Instance<'tcx>,
    owner: LocalDefId,
    request_span: Span,
    require_provider: bool,
) -> Result<Vec<Marker<'tcx>>, String> {
    let owner = tcx.parent_module_from_def_id(owner).to_local_def_id();
    let definition = instance.def_id();
    if !tcx.is_mir_available(definition) {
        return Err(format!(
            "external provider blueprint {} has no encoded compiler metadata; rebuild its crate with compatible cargo nestrs",
            tcx.def_path_str(definition),
        ));
    }
    let mut markers = Vec::new();
    for call in crate::registration_codegen::descriptor_calls(tcx, instance)? {
        let (callee, generic_args, args, body) =
            (call.definition, call.arguments, call.operands, call.body);
        let Some(kind) = marker_kind(tcx, callee) else {
            continue;
        };
        let key = if kind == MarkerKind::Provider {
            let [argument] = args else {
                return Err(
                    "external compiler_provider metadata has an incompatible argument count".into(),
                );
            };
            Some(external_key(tcx, body, &argument.node)?)
        } else {
            None
        };
        markers.push(Marker {
            kind,
            types: generic_args.types().collect(),
            owner,
            span: request_span,
            key,
            passive: false,
        });
    }
    if require_provider
        && !markers
            .iter()
            .any(|marker| marker.kind == MarkerKind::Provider)
    {
        return Err(format!(
            "external provider blueprint {} is missing preserved Nestrs compiler markers; rebuild the dependency with compatible cargo nestrs and nestrs-core",
            tcx.def_path_str(definition),
        ));
    }
    Ok(markers)
}

/// The generated key is a literal enum value. Accept exactly constants and
/// uniquely assigned local aliases/aggregates; an arbitrary hand-written body
/// must never make this analysis guess among control-flow-dependent values.
pub(crate) fn external_key<'tcx>(
    tcx: TyCtxt<'tcx>,
    body: &mir::Body<'tcx>,
    argument: &mir::Operand<'tcx>,
) -> Result<CompilerKey, String> {
    let mut operand = argument;
    let mut visited = HashSet::new();
    loop {
        match operand {
            mir::Operand::Constant(constant) => {
                let ty = constant.const_.ty();
                let value = constant
                    .const_
                    .eval(tcx, ty::TypingEnv::fully_monomorphized(), constant.span)
                    .map_err(|_| "cannot evaluate external CompilerKey constant".to_string())?;
                let ty::Adt(def, _) = ty.kind() else {
                    return Err("external CompilerKey constant is not an enum".into());
                };
                let data = tcx
                    .try_destructure_mir_constant_for_user_output(value, ty)
                    .ok_or_else(|| "cannot inspect external CompilerKey constant".to_string())?;
                let variant = data
                    .variant
                    .ok_or_else(|| "external CompilerKey constant lacks a variant".to_string())?;
                return external_key_parts(tcx, def.variants()[variant].def_id, data.fields);
            }
            mir::Operand::Copy(place) | mir::Operand::Move(place) => {
                if !place.projection.is_empty() || !visited.insert(place.local) {
                    return Err(
                        "external CompilerKey metadata has a projected or cyclic operand".into(),
                    );
                }
                let assignments: Vec<_> = body
                    .basic_blocks
                    .iter()
                    .flat_map(|block| &block.statements)
                    .filter_map(|statement| match &statement.kind {
                        mir::StatementKind::Assign(assignment) if assignment.0 == *place => {
                            Some(&assignment.1)
                        }
                        _ => None,
                    })
                    .collect();
                let [assignment] = assignments.as_slice() else {
                    return Err("external CompilerKey metadata must have one unambiguous literal assignment".into());
                };
                match assignment {
                    mir::Rvalue::Use(value, _) => operand = value,
                    mir::Rvalue::Aggregate(kind, operands) => {
                        let mir::AggregateKind::Adt(def_id, variant, _, _, _) = **kind else {
                            return Err("external CompilerKey aggregate is not an enum".into());
                        };
                        let mut fields = Vec::new();
                        for operand in operands {
                            let mir::Operand::Constant(constant) = operand else {
                                return Err("external CompilerKey payload must remain a declaration literal".into());
                            };
                            let value = constant
                                .const_
                                .eval(tcx, ty::TypingEnv::fully_monomorphized(), constant.span)
                                .map_err(|_| {
                                    "cannot evaluate external CompilerKey payload".to_string()
                                })?;
                            fields.push((value, constant.const_.ty()));
                        }
                        return external_key_parts(
                            tcx,
                            tcx.adt_def(def_id).variants()[variant].def_id,
                            &fields,
                        );
                    }
                    _ => {
                        return Err(
                            "external CompilerKey metadata is not a literal enum constructor"
                                .into(),
                        );
                    }
                }
            }
            _ => return Err("external CompilerKey metadata has an unsupported operand".into()),
        }
    }
}

fn external_key_parts<'tcx>(
    tcx: TyCtxt<'tcx>,
    variant: DefId,
    fields: &[(mir::ConstValue, Ty<'tcx>)],
) -> Result<CompilerKey, String> {
    if !reflect_item(tcx, tcx.parent(variant), protocol::COMPILER_KEY) {
        return Err("external CompilerKey variant comes from an unexpected crate".into());
    }
    match (tcx.item_name(variant).as_str(), fields) {
        ("Default", []) => Ok(CompilerKey::Default),
        ("Indexed", [(value, _)]) => value
            .try_to_target_usize(tcx)
            .map(|value| CompilerKey::Indexed(value as u128))
            .ok_or_else(|| "external CompilerKey indexed payload is invalid".into()),
        ("Named", [(value, _)]) => {
            let bytes = value
                .try_get_slice_bytes_for_diagnostics(tcx)
                .ok_or_else(|| "external CompilerKey named payload is invalid".to_string())?;
            let value = std::str::from_utf8(bytes)
                .map_err(|_| "external CompilerKey named payload is not UTF-8".to_string())?;
            Ok(CompilerKey::Named(value.to_owned()))
        }
        _ => Err("external CompilerKey metadata has an incompatible variant or payload".into()),
    }
}

/// Capabilities come from real business trait impls, including upstream blanket
/// impls. A closed provider fixes the impl's arguments; unconstrained trait
/// parameters are never guessed.
fn capability_impls(tcx: TyCtxt<'_>) -> Vec<DefId> {
    let mut implementations = HashSet::new();
    for definition in tcx.iter_local_def_id() {
        if tcx.def_kind(definition) == (DefKind::Impl { of_trait: true }) {
            implementations.insert(definition.to_def_id());
        }
    }
    // External blanket impls may apply to a private local provider without a
    // corresponding local impl item. Enumerate real business-trait metadata,
    // then let rustc solve each finite concrete/impl pair below.
    for &krate in tcx.crates(()) {
        if matches!(
            tcx.crate_name(krate).as_str(),
            "core" | "std" | "alloc" | "nestrs_core"
        ) {
            continue;
        }
        for &trait_id in tcx.traits(krate) {
            if tcx.is_dyn_compatible(trait_id) && !tcx.trait_is_auto(trait_id) {
                implementations.extend(tcx.all_impls(trait_id));
            }
        }
    }
    let mut implementations: Vec<_> = implementations.into_iter().collect();
    implementations.sort_by_key(|definition| tcx.def_path_str(*definition));
    implementations
}

fn declared_interfaces<'tcx>(
    tcx: TyCtxt<'tcx>,
    concrete: Ty<'tcx>,
    implementations: &[DefId],
) -> HashSet<Ty<'tcx>> {
    let mut interfaces = HashSet::new();
    for &definition in implementations {
        let raw = tcx
            .impl_trait_ref(definition)
            .instantiate_identity()
            .skip_normalization();
        if matches!(
            tcx.crate_name(raw.def_id.krate).as_str(),
            "core" | "std" | "alloc" | "nestrs_core"
        ) {
            continue;
        }
        let infcx = tcx
            .infer_ctxt()
            .ignoring_regions()
            .build(ty::TypingMode::non_body_analysis());
        let args = infcx.fresh_args_for_item(tcx.def_span(definition), definition);
        let ocx = ObligationCtxt::new(&infcx);
        let cause = ObligationCause::dummy();
        let environment = ty::ParamEnv::empty();
        let self_ty = ocx.normalize(
            &cause,
            environment,
            tcx.type_of(definition).instantiate(tcx, args),
        );
        if ocx.eq(&cause, environment, concrete, self_ty).is_err() {
            continue;
        }
        let trait_ref = ocx.normalize(
            &cause,
            environment,
            tcx.impl_trait_ref(definition).instantiate(tcx, args),
        );
        ocx.register_obligation(Obligation::new(tcx, cause, environment, trait_ref));
        if !ocx.evaluate_obligations_error_on_ambiguity().is_empty() {
            continue;
        }
        let trait_ref = infcx.resolve_vars_if_possible(trait_ref);
        if trait_ref.has_infer()
            || trait_ref.has_non_region_param()
            || trait_ref.has_escaping_bound_vars()
        {
            continue;
        }
        for supertrait in traits::supertraits(tcx, ty::Binder::dummy(trait_ref)) {
            let Some(supertrait) = supertrait.no_bound_vars() else {
                continue;
            };
            if !matches!(
                tcx.crate_name(supertrait.def_id.krate).as_str(),
                "core" | "std" | "alloc" | "nestrs_core"
            ) && !tcx.trait_is_auto(supertrait.def_id)
                && tcx.is_dyn_compatible(supertrait.def_id)
            {
                interfaces.extend(interface_variants(tcx, concrete, supertrait));
            }
        }
    }
    interfaces
}

fn interface_variants<'tcx>(
    tcx: TyCtxt<'tcx>,
    concrete: Ty<'tcx>,
    principal: ty::TraitRef<'tcx>,
) -> Vec<Ty<'tcx>> {
    let mut predicates = vec![ty::Binder::dummy(ty::ExistentialPredicate::Trait(
        ty::ExistentialTraitRef::erase_self_ty(tcx, principal),
    ))];
    let infcx = tcx
        .infer_ctxt()
        .ignoring_regions()
        .build(ty::TypingMode::non_body_analysis());
    let ocx = ObligationCtxt::new(&infcx);
    let clause: ty::Clause<'tcx> = principal.upcast(tcx);
    let implied: Vec<_> =
        traits::elaborate(tcx, [clause])
            .filter_only_self()
            .filter_map(|clause| clause.as_projection_clause())
            .map(|projection| {
                tcx.erase_and_anonymize_regions(projection.map_bound(|projection| {
                    ty::ExistentialProjection::erase_self_ty(tcx, projection)
                }))
            })
            .collect();
    for supertrait in traits::supertraits(tcx, ty::Binder::dummy(principal)) {
        for item in tcx
            .associated_items(supertrait.skip_binder().def_id)
            .in_definition_order()
        {
            if !matches!(tcx.def_kind(item.def_id), DefKind::AssocTy)
                || tcx.generics_require_sized_self(item.def_id)
            {
                continue;
            }
            if tcx.generics_of(item.def_id).count() != supertrait.skip_binder().args.len() {
                return Vec::new();
            }
            // A supertrait may quantify lifetimes independently of the
            // principal. Normalize inside that binder: erasing its lifetimes
            // would conflate e.g. View<'a>::Item = &'a str with a static item.
            let projection = supertrait.map_bound(|trait_ref| {
                Ty::new_projection(tcx, ty::IsRigid::No, item.def_id, trait_ref.args)
            });
            let value = ocx.normalize(
                &ObligationCause::dummy(),
                ty::ParamEnv::empty(),
                ty::Unnormalized::new_wip(projection),
            );
            if !ocx.evaluate_obligations_error_on_ambiguity().is_empty() {
                return Vec::new();
            }
            let value = infcx.resolve_vars_if_possible(value);
            if value.has_infer()
                || value.has_non_region_param()
                || value.has_escaping_bound_vars()
                || matches!(value.skip_binder().kind(), ty::Alias(..))
            {
                return Vec::new();
            }
            let binding = supertrait.rebind(ty::ExistentialProjection::new(
                tcx,
                item.def_id,
                supertrait.skip_binder().args.iter().skip(1),
                value.skip_binder().into(),
            ));
            // A lifetime introduced only by a supertrait cannot be written in
            // the principal's associated-type arguments. An equality already
            // implied by that principal is representable (and rustc's typed
            // source printer omits it); do not invent a lifetime or substitute
            // 'static for other, unnameable, higher-ranked projections.
            if binding.skip_binder().term.has_escaping_bound_vars()
                && !implied.contains(&tcx.erase_and_anonymize_regions(binding))
            {
                return Vec::new();
            }
            predicates.push(binding.map_bound(ty::ExistentialPredicate::Projection));
        }
    }
    let (Some(send), Some(sync), Some(unsize)) = (
        tcx.get_diagnostic_item(rustc_span::sym::Send),
        tcx.lang_items().sync_trait(),
        tcx.lang_items().unsize_trait(),
    ) else {
        return Vec::new();
    };
    let mut variants = Vec::new();
    for mask in 0..4 {
        let mut predicates = predicates.clone();
        if mask & 1 != 0 {
            predicates.push(ty::Binder::dummy(ty::ExistentialPredicate::AutoTrait(send)));
        }
        if mask & 2 != 0 {
            predicates.push(ty::Binder::dummy(ty::ExistentialPredicate::AutoTrait(sync)));
        }
        use rustc_middle::ty::ExistentialPredicateStableCmpExt as _;
        predicates.sort_by(|left, right| left.skip_binder().stable_cmp(tcx, &right.skip_binder()));
        predicates.dedup();
        let predicates = tcx.mk_poly_existential_predicates_from_iter(predicates.into_iter());
        let object = Ty::new_dynamic(tcx, predicates, tcx.lifetimes.re_static);
        if [send, sync].into_iter().all(|bound| {
            infcx
                .type_implements_trait(bound, [object], ty::ParamEnv::empty())
                .must_apply_modulo_regions()
        }) && infcx
            .type_implements_trait(unsize, [concrete, object], ty::ParamEnv::empty())
            .must_apply_modulo_regions()
        {
            variants.push(tcx.erase_and_anonymize_regions(object));
        }
    }
    variants
}

fn insertion_for<'tcx>(
    tcx: TyCtxt<'tcx>,
    concrete: Ty<'tcx>,
    interface: Ty<'tcx>,
    fallback_owner: LocalDefId,
    binding: BindingSpec,
) -> Result<SourceInsertion, String> {
    let mut insertion = source_insertion(tcx, concrete, Some(interface), fallback_owner)?;
    insertion.bindings.push(binding);
    Ok(insertion)
}

fn source_insertion<'tcx>(
    tcx: TyCtxt<'tcx>,
    concrete: Ty<'tcx>,
    interface: Option<Ty<'tcx>>,
    fallback_owner: LocalDefId,
) -> Result<SourceInsertion, String> {
    // A query callback is nested inside a function, but its closed service
    // need not be block-local. Use the request's containing module; the actual
    // service declaration below still rejects genuinely block-local types.
    let fallback_owner = tcx
        .parent_module_from_def_id(fallback_owner)
        .to_local_def_id();
    let concrete_owner = match concrete.kind() {
        ty::Adt(definition, _) => definition.did().as_local().unwrap_or(fallback_owner),
        _ => fallback_owner,
    };
    let mut choices = vec![module_for(tcx, concrete_owner, concrete)?];
    choices.push(module_for(tcx, fallback_owner, concrete)?);
    for owner in tcx.iter_local_def_id() {
        if tcx.def_kind(owner) != (DefKind::Impl { of_trait: true }) {
            continue;
        }
        let self_ty = tcx
            .type_of(owner)
            .instantiate_identity()
            .skip_normalization();
        let same_family = match (concrete.kind(), self_ty.kind()) {
            (ty::Adt(left, _), ty::Adt(right, _)) => left.did() == right.did(),
            _ => concrete == self_ty,
        };
        if same_family && let Ok(module) = module_for(tcx, owner, concrete) {
            // Unrelated block-local impls do not invalidate a legal module.
            choices.push(module);
        }
    }
    let owner = choices
        .into_iter()
        .find(|module| {
            let (hir_module, span, _) = tcx.hir_get_module(LocalModDefId::new_unchecked(*module));
            !span.from_expansion()
                && !hir_module.spans.inject_use_span.from_expansion()
                && crate::type_source::accessible(tcx, concrete, *module)
                && interface.is_none_or(|interface| crate::type_source::accessible(tcx, interface, *module))
        })
        .ok_or_else(|| {
            format!(
                "no supported source module in the current crate can name {} for a legal automatic binding or blueprint; private upstream types require an existing producer capability for this exact interface shape; macro-generated modules require a compiler hygiene bridge",
                interface.map_or_else(|| concrete.to_string(), |interface| format!("both {concrete} and {interface}")),
            )
        })?;
    let (module, span, _) = tcx.hir_get_module(LocalModDefId::new_unchecked(owner));
    // This is the parser's legal item-insertion point, after inner attributes.
    // On this pinned compiler inner_span.hi() includes the closing brace for
    // an inline module and would accidentally place generated items outside.
    let inside = module.spans.inject_use_span;
    if span.from_expansion() || inside.from_expansion() {
        return Err(format!(
            "cannot insert automatic binding for {concrete} into a macro-generated module"
        ));
    }
    let source = tcx.sess.source_map().lookup_source_file(inside.lo());
    let FileName::Real(name) = &source.name else {
        return Err(format!(
            "automatic binding for {concrete} has no physical source file"
        ));
    };
    let path = name
        .local_path()
        .ok_or_else(|| format!("source path for {concrete} was remapped without a local path"))?
        .to_path_buf();
    let path = crate::documentation::remap_source_path(&path);
    let expected_source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read source {}: {error}", path.display()))?;
    let offset = source.original_relative_byte_pos(inside.lo()).0 as usize;
    Ok(SourceInsertion {
        path,
        offset,
        expected_source,
        bindings: vec![],
    })
}

fn module_for(
    tcx: TyCtxt<'_>,
    mut owner: LocalDefId,
    concrete: Ty<'_>,
) -> Result<LocalDefId, String> {
    let initial = owner;
    loop {
        if tcx.def_kind(owner) == DefKind::Mod {
            return Ok(owner);
        }
        // A block-local declaration cannot be named by a generated module item.
        if matches!(tcx.def_kind(owner), DefKind::Fn | DefKind::AssocFn) && owner != initial {
            return Err(format!(
                "automatic binding for block-local type {concrete} is unsupported"
            ));
        }
        owner = tcx.local_parent(owner);
    }
}
