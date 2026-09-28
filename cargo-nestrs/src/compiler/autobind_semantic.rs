//! Compiler-owned discovery for the automatic binding experiment.
//!
//! The three hidden marker functions describe DI intent. Types and candidate
//! applicability come from rustc; source text is only retained for the final
//! source overlay, never searched for services or impls.

extern crate rustc_ast;
extern crate rustc_infer;
extern crate rustc_span;
extern crate rustc_trait_selection;

use crate::autobind_codegen::{BindingSpec, SourceInsertion};
use rustc_hir::def::DefKind;
use rustc_hir::def_id::{DefId, DefIndex, LocalDefId, LocalModDefId};
use rustc_hir::intravisit::{self, Visitor};
use rustc_infer::infer::TyCtxtInferExt;
use rustc_middle::mir;
use rustc_middle::ty::{self, Ty, TyCtxt, TypeVisitableExt};
use rustc_span::{FileName, Span};
use rustc_trait_selection::infer::InferCtxtExt;
use std::collections::{HashMap, HashSet, VecDeque};

pub struct Analysis {
    pub insertions: Vec<SourceInsertion>,
    pub providers: usize,
    pub requests: usize,
    pub generated_bindings: usize,
    pub explicit_bindings: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MarkerKind {
    Provider,
    Request,
    Binding,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum CompilerKey {
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
            && self.tcx.crate_name(def_id.krate).as_str() == "nestrs_core"
        {
            let kind = match definition_path(self.tcx, def_id).as_str() {
                "registration::compiler::compiler_provider" => Some(MarkerKind::Provider),
                "registration::compiler::compiler_request" => Some(MarkerKind::Request),
                "registration::compiler::compiler_binding" => Some(MarkerKind::Binding),
                _ => None,
            };
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
        if self.tcx.crate_name(definition.krate).as_str() != "nestrs_core" {
            return Err("CompilerKey marker variant belongs to an unexpected crate".into());
        }
        match (definition_path(self.tcx, definition).as_str(), literal) {
            ("registration::compiler::CompilerKey::Default", None) => Ok(CompilerKey::Default),
            (
                "registration::compiler::CompilerKey::Named",
                Some(rustc_ast::LitKind::Str(name, _)),
            ) => Ok(CompilerKey::Named(name.to_string())),
            (
                "registration::compiler::CompilerKey::Indexed",
                Some(rustc_ast::LitKind::Int(value, _)),
            ) => Ok(CompilerKey::Indexed(value.0)),
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

fn type_source(ty: Ty<'_>) -> String {
    rustc_middle::ty::print::with_no_trimmed_paths!(rustc_middle::ty::print::with_crate_prefix!(
        ty.to_string()
    ))
}

fn definition_path(tcx: TyCtxt<'_>, definition: DefId) -> String {
    tcx.def_path(definition)
        .data
        .iter()
        .map(|component| {
            component
                .data
                .get_opt_name()
                .map_or_else(|| String::from("<anonymous>"), |name| name.to_string())
        })
        .collect::<Vec<_>>()
        .join("::")
}

fn provider_definition(tcx: TyCtxt<'_>) -> Option<(DefId, DefId)> {
    for crate_num in tcx.crates(()) {
        if tcx.crate_name(*crate_num).as_str() != "nestrs_core" {
            continue;
        }
        for trait_id in tcx.traits(*crate_num).iter().copied() {
            if definition_path(tcx, trait_id) == "registration::provider::ProviderDefinition" {
                let method = tcx
                    .associated_items(trait_id)
                    .in_definition_order()
                    .find(|item| item.name().as_str() == "provider")?;
                return Some((trait_id, method.def_id));
            }
        }
    }
    None
}

/// Analyze only this crate's DI declarations. Dependencies must be processed
/// by the framework wrapper separately; this phase does not rewrite a foreign
/// crate or manufacture registrations for arbitrary external implementations.
pub fn analyze(tcx: TyCtxt<'_>) -> Result<Analysis, String> {
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

    let mut by_owner: HashMap<LocalDefId, Vec<Marker<'_>>> = HashMap::new();
    let mut pending = VecDeque::new();
    let mut explicit_providers = HashSet::new();
    for marker in markers {
        // A closed-looking field inside an unused generic blueprint is not a
        // global DI request. Its markers become live only when that blueprint
        // is instantiated through a known closed root or dependency.
        if tcx.generics_of(marker.owner).count() == 0 && marker.types.iter().copied().all(closed) {
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
        by_owner.entry(marker.owner).or_default().push(marker);
    }

    let infcx = tcx
        .infer_ctxt()
        .ignoring_regions()
        .build(ty::TypingMode::non_body_analysis());
    let param_env = ty::ParamEnv::empty();
    let definition = provider_definition(tcx);
    let mut providers: HashMap<Ty<'_>, Marker<'_>> = HashMap::new();
    let mut requests = HashSet::new();
    let mut bindings = HashSet::new();
    let mut expanded = HashSet::new();
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
            });
        }
    }

    loop {
        if pending.is_empty() {
            for marker in &closed_impls {
                let concrete = marker.types[0];
                if !expanded.contains(&concrete)
                    && requests.iter().any(|interface| {
                        infcx
                            .type_implements_trait(unsize_trait, [concrete, *interface], param_env)
                            .must_apply_modulo_regions()
                    })
                {
                    pending.push_back(marker.clone());
                }
            }
        }
        let Some(mut marker) = pending.pop_front() else {
            break;
        };
        for ty in &mut marker.types {
            *ty = normalized(tcx, marker.owner, *ty)?;
        }
        if !marker.types.iter().copied().all(closed) {
            return Err("DI marker remained generic after closed-type expansion".into());
        }
        match marker.kind {
            MarkerKind::Provider => {
                providers.entry(marker.types[0]).or_insert(marker);
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
                    requests.insert(requested);
                    continue;
                }
                if !expanded.insert(requested) {
                    continue;
                }
                let Some((trait_id, method_id)) = definition else {
                    continue;
                };
                if !infcx
                    .type_implements_trait(trait_id, [requested], param_env)
                    .must_apply_modulo_regions()
                {
                    continue;
                }
                let args = tcx.mk_args(&[requested.into()]);
                let instance = ty::Instance::try_resolve(
                    tcx,
                    ty::TypingEnv::fully_monomorphized(),
                    method_id,
                    args,
                )
                .map_err(|_| format!("rustc could not resolve provider blueprint for {requested}"))?
                .ok_or_else(|| {
                    format!("provider blueprint for {requested} is not fully resolved")
                })?;
                let instantiated_markers = if let Some(owner) = instance.def_id().as_local() {
                    let nested = by_owner.get(&owner).ok_or_else(|| {
                        format!("provider blueprint {requested} has no compiler markers; rebuild it with compatible Nestrs tooling")
                    })?;
                    let mut instantiated_markers = Vec::new();
                    for nested in nested {
                        let mut instantiated = nested.clone();
                        for ty in &mut instantiated.types {
                            *ty = ty::EarlyBinder::bind(tcx, *ty)
                                .instantiate(tcx, instance.args)
                                .skip_normalization();
                        }
                        instantiated_markers.push(instantiated);
                    }
                    instantiated_markers
                } else {
                    external_blueprint_markers(tcx, instance, marker.owner, marker.span)?
                };
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
                pending.extend(instantiated_markers);
            }
        }
    }

    let inherited_bindings = inherited_bindings(tcx)?;
    let mut insertions = Vec::new();
    let mut generated = 0;
    let mut candidates: Vec<_> = providers.iter().collect();
    candidates.sort_by_key(|(ty, _)| type_source(**ty));
    let mut interfaces: Vec<_> = requests.iter().copied().collect();
    interfaces.sort_by_key(|ty| type_source(*ty));
    for (concrete, marker) in candidates {
        for interface in &interfaces {
            if bindings.contains(&(*concrete, *interface))
                || inherited_bindings.contains(&(*concrete, *interface))
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
            let binding = BindingSpec {
                concrete: type_source(*concrete),
                interface: type_source(*interface),
                source_file: source.file.name.prefer_local_unconditionally().to_string(),
                source_line: source.line as u32,
                source_column: source.col.0 as u32 + 1,
            };
            insertions.push(insertion_for(
                tcx,
                *concrete,
                *interface,
                marker.owner,
                binding,
            )?);
            generated += 1;
        }
    }
    Ok(Analysis {
        insertions,
        providers: providers.len(),
        requests: requests.len(),
        generated_bindings: generated,
        explicit_bindings: bindings.len(),
    })
}

/// Generated registration callbacks use this private ABI name in both explicit
/// macro fixtures and automatically emitted code. Read their *typed marker
/// calls*, not name-based trait guesses, from upstream metadata. An inherited
/// pair prevents a second automatic callback; existing explicit duplicates are
/// left in the linked registration slices for the graph compiler to diagnose.
fn inherited_bindings<'tcx>(tcx: TyCtxt<'tcx>) -> Result<HashSet<(Ty<'tcx>, Ty<'tcx>)>, String> {
    let mut bindings = HashSet::new();
    for &krate in tcx.crates(()) {
        for index in 0..tcx.num_extern_def_ids(krate) {
            let definition = DefId {
                krate,
                index: DefIndex::from_usize(index),
            };
            // Metadata def-index tables may contain holes for stripped/private
            // items. MIR table availability is safe to probe before asking for
            // the corresponding DefKey or function signature.
            if !tcx.is_mir_available(definition) {
                continue;
            }
            if tcx
                .opt_item_name(definition)
                .is_none_or(|name| name.as_str() != "__nestrs_reflect_trait_binding")
            {
                continue;
            }
            for block in tcx.optimized_mir(definition).basic_blocks.iter() {
                let mir::TerminatorKind::Call { func, .. } = &block.terminator().kind else {
                    continue;
                };
                let body = tcx.optimized_mir(definition);
                let ty::FnDef(callee, arguments) = *func.ty(&body.local_decls, tcx).kind() else {
                    continue;
                };
                if tcx.crate_name(callee.krate).as_str() != "nestrs_core"
                    || definition_path(tcx, callee) != "registration::compiler::compiler_binding"
                {
                    continue;
                }
                let types: Vec<_> = arguments.types().collect();
                let [concrete, interface] = types.as_slice() else {
                    return Err(
                        "upstream compiler_binding marker has an incompatible type-argument count"
                            .into(),
                    );
                };
                if !closed(*concrete) || !closed(*interface) {
                    return Err(
                        "upstream binding callback unexpectedly contains an open generic type"
                            .into(),
                    );
                }
                let concrete = tcx
                    .try_normalize_erasing_regions(
                        ty::TypingEnv::fully_monomorphized(),
                        ty::Unnormalized::new_wip(*concrete),
                    )
                    .map_err(|error| {
                        format!("cannot normalize upstream binding concrete type: {error:?}")
                    })?;
                let interface = tcx
                    .try_normalize_erasing_regions(
                        ty::TypingEnv::fully_monomorphized(),
                        ty::Unnormalized::new_wip(*interface),
                    )
                    .map_err(|error| {
                        format!("cannot normalize upstream binding interface type: {error:?}")
                    })?;
                bindings.insert((concrete, interface));
            }
        }
    }
    Ok(bindings)
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
) -> Result<Vec<Marker<'tcx>>, String> {
    let owner = tcx.parent_module_from_def_id(owner).to_local_def_id();
    let definition = instance.def_id();
    if !tcx.is_mir_available(definition) {
        return Err(format!(
            "external provider blueprint {} has no encoded compiler metadata; rebuild its crate with compatible cargo nestrs",
            tcx.def_path_str(definition),
        ));
    }
    let body = tcx.optimized_mir(definition);
    let mut markers = Vec::new();
    for block in body.basic_blocks.iter() {
        let mir::TerminatorKind::Call { func, args, .. } = &block.terminator().kind else {
            continue;
        };
        let ty::FnDef(callee, generic_args) = *func.ty(&body.local_decls, tcx).kind() else {
            continue;
        };
        if tcx.crate_name(callee.krate).as_str() != "nestrs_core" {
            continue;
        }
        let kind = match definition_path(tcx, callee).as_str() {
            "registration::compiler::compiler_provider" => MarkerKind::Provider,
            "registration::compiler::compiler_request" => MarkerKind::Request,
            "registration::compiler::compiler_binding" => MarkerKind::Binding,
            _ => continue,
        };
        let generic_args = ty::EarlyBinder::bind(tcx, generic_args)
            .instantiate(tcx, instance.args)
            .skip_normalization();
        let key = if kind == MarkerKind::Provider {
            let [argument] = args.as_ref() else {
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
        });
    }
    if !markers
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
fn external_key<'tcx>(
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
    if tcx.crate_name(variant.krate).as_str() != "nestrs_core" {
        return Err("external CompilerKey variant comes from an unexpected crate".into());
    }
    match (definition_path(tcx, variant).as_str(), fields) {
        ("registration::compiler::CompilerKey::Default", []) => Ok(CompilerKey::Default),
        ("registration::compiler::CompilerKey::Indexed", [(value, _)]) => value
            .try_to_target_usize(tcx)
            .map(|value| CompilerKey::Indexed(value as u128))
            .ok_or_else(|| "external CompilerKey indexed payload is invalid".into()),
        ("registration::compiler::CompilerKey::Named", [(value, _)]) => {
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

fn insertion_for<'tcx>(
    tcx: TyCtxt<'tcx>,
    concrete: Ty<'tcx>,
    interface: Ty<'tcx>,
    fallback_owner: LocalDefId,
    binding: BindingSpec,
) -> Result<SourceInsertion, String> {
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
                && accessible(tcx, concrete, *module)
                && accessible(tcx, interface, *module)
        })
        .ok_or_else(|| {
            format!(
                "no supported source module can name both {} and {} for a legal automatic binding; macro-generated modules require a compiler hygiene bridge",
                type_source(concrete),
                type_source(interface)
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
    let expected_source = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read source {}: {error}", path.display()))?;
    let offset = source.original_relative_byte_pos(inside.lo()).0 as usize;
    Ok(SourceInsertion {
        path,
        offset,
        expected_source,
        bindings: vec![binding],
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

fn accessible(tcx: TyCtxt<'_>, ty: Ty<'_>, module: LocalDefId) -> bool {
    let visible = |mut definition: DefId| {
        loop {
            if !tcx.visibility(definition).is_accessible_from(module, tcx) {
                return false;
            }
            // The type printer uses rustc's public reexport graph for foreign
            // definitions. Check that same graph, not a private definition-site
            // module which may not occur in the emitted type path at all.
            let parent = if definition.is_local() {
                tcx.opt_parent(definition)
            } else {
                tcx.visible_parent_map(())
                    .get(&definition)
                    .copied()
                    .or_else(|| tcx.opt_parent(definition))
            };
            let Some(parent) = parent else {
                return true;
            };
            definition = parent;
            if tcx.def_kind(definition) != DefKind::Mod {
                return false;
            }
        }
    };
    for argument in ty.walk() {
        if let ty::GenericArgKind::Type(nested) = argument.kind() {
            match nested.kind() {
                ty::Adt(definition, _) if !visible(definition.did()) => return false,
                ty::Dynamic(predicates, _)
                    if predicates
                        .principal_def_id()
                        .is_some_and(|trait_id| !visible(trait_id)) =>
                {
                    return false;
                }
                _ => {}
            }
        }
    }
    true
}
