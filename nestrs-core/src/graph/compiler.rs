use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use crate::{
    ServiceLifetime,
    registration::{
        binding::{REFLECTED_AUTOMATIC_BINDINGS, REFLECTED_BINDINGS, TraitBinding},
        dependency::{ClosedProviderCallback, Delivery, DependencyRequest, ProviderSource},
        provider::{Provider, ProviderCommon, REFLECTED_PROVIDERS},
        root::{REFLECTED_ROOTS, RootDeclaration},
    },
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};

use super::{
    CompiledDependency, CompiledNode, Constructor, GraphDiagnostic, GraphDiagnosticKind as Kind,
    GraphError, ProviderId, RootRoute, ValidatedGraph,
};

pub(crate) struct GraphCompiler;

struct Declaration {
    identifier: ServiceIdentifier,
    common: ProviderCommon,
    dependencies: Vec<DependencyRequest>,
    constructor: Constructor,
}

impl From<Provider> for Declaration {
    fn from(provider: Provider) -> Self {
        match provider {
            Provider::Class(provider) => Self {
                identifier: provider.provide,
                common: provider.common,
                dependencies: provider.dependencies,
                constructor: Constructor::Class(provider.constructor),
            },
            Provider::Factory(provider) => Self {
                identifier: provider.provide,
                common: provider.common,
                dependencies: provider.dependencies,
                constructor: Constructor::Factory(provider.invoker),
            },
        }
    }
}

impl GraphCompiler {
    pub(crate) fn compile_static() -> Result<ValidatedGraph, GraphError> {
        Self::compile_with_automatic_bindings(
            REFLECTED_PROVIDERS
                .iter()
                .map(|declare| declare())
                .collect(),
            REFLECTED_BINDINGS.iter().map(|declare| declare()).collect(),
            REFLECTED_ROOTS.iter().map(|declare| declare()).collect(),
            REFLECTED_AUTOMATIC_BINDINGS
                .iter()
                .map(|declare| declare())
                .collect(),
        )
    }

    /// An owned snapshot lets graph tests stay isolated from process-wide linkme slices.
    #[cfg(test)]
    pub(crate) fn compile_snapshot(
        providers: Vec<Provider>,
        bindings: Vec<TraitBinding>,
        roots: Vec<RootDeclaration>,
    ) -> Result<ValidatedGraph, GraphError> {
        Self::compile_with_automatic_bindings(providers, bindings, roots, vec![])
    }

    pub(crate) fn compile_with_automatic_bindings(
        providers: Vec<Provider>,
        mut bindings: Vec<TraitBinding>,
        mut roots: Vec<RootDeclaration>,
        mut automatic_bindings: Vec<TraitBinding>,
    ) -> Result<ValidatedGraph, GraphError> {
        let mut diagnostics = Vec::new();
        let mut declarations: Vec<Declaration> = providers.into_iter().map(Into::into).collect();
        sort_declarations(&mut declarations);

        // Explicit duplicates are errors. Materialized declarations use a different insertion
        // path, so a repeated closed root never hides two conflicting explicit registrations.
        let mut catalog = HashMap::new();
        let mut unique: Vec<Declaration> = Vec::new();
        for declaration in declarations {
            if let Some(&existing) = catalog.get(&declaration.identifier) {
                let original: &Declaration = &unique[existing];
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::DuplicateProvider,
                        format!(
                            "重复 Provider {}：{} 与 {}（primary 不能覆盖 concrete 注册）",
                            token(&declaration.identifier),
                            source(original.common.source),
                            source(declaration.common.source),
                        ),
                    )
                    .at(&original.identifier, original.common.source)
                    .at(&declaration.identifier, declaration.common.source),
                );
            } else {
                catalog.insert(declaration.identifier.clone(), unique.len());
                unique.push(declaration);
            }
        }
        let mut declarations = unique;
        let explicit_count = declarations.len();

        // A projection exported by a dependency is a capability, not a request to register
        // every interface it implements. Activate it only when the linked graph needs that
        // interface. Sibling crates may emit the same automatic pair independently; select
        // one deterministically without hiding duplicate *explicit* declarations below.
        automatic_bindings.sort_by_key(binding_order);
        let mut known_pairs: HashSet<_> = bindings
            .iter()
            .map(|binding| (binding.trait_type, binding.concrete_type))
            .collect();
        let mut automatic_by_interface: HashMap<ServiceType, Vec<TraitBinding>> = HashMap::new();
        for binding in automatic_bindings {
            if known_pairs.insert((binding.trait_type, binding.concrete_type)) {
                automatic_by_interface
                    .entry(binding.trait_type)
                    .or_default()
                    .push(binding);
            }
        }
        let mut requested_interfaces: VecDeque<_> =
            roots.iter().map(|root| root.service_type).collect();
        let mut visited_interfaces = HashSet::new();

        // A closed bind self type is also a declaration-time anchor. Querying only dyn Trait
        // must discover its generic concrete provider before orphan-binding validation.
        roots.extend(bindings.iter().filter_map(|binding| {
            binding.materialize.map(|callback| RootDeclaration {
                service_type: binding.concrete_type,
                materialize: Some(callback),
                source: binding.source,
            })
        }));
        roots.sort_by_key(|root| (root.service_type.name, root.source));
        let mut materialized = HashSet::new();
        let mut declared_roots = HashSet::new();
        for root in roots {
            // A factory/trait/optional query may carry no definition. It must not consume
            // this type's deduplication entry before another query supplies a callback.
            let Some(callback) = root.materialize else {
                continue;
            };
            if !declared_roots.insert(root.service_type) {
                continue;
            }
            materialize(
                root.service_type,
                callback,
                root.source,
                &mut materialized,
                &mut declarations,
                &mut catalog,
                explicit_count,
                &mut diagnostics,
            );
        }

        // Two work queues close both concrete blueprints and demanded interface projections.
        // An activated projection may reveal a generic provider whose dependencies demand
        // another interface. Every descriptor and interface is examined once, without recursion.
        let mut cursor = 0;
        while cursor < declarations.len() || !requested_interfaces.is_empty() {
            if let Some(interface) = requested_interfaces.pop_front() {
                if visited_interfaces.insert(interface)
                    && let Some(automatic) = automatic_by_interface.remove(&interface)
                {
                    for binding in automatic {
                        if let Some(callback) = binding.materialize {
                            materialize(
                                binding.concrete_type,
                                callback,
                                binding.source,
                                &mut materialized,
                                &mut declarations,
                                &mut catalog,
                                explicit_count,
                                &mut diagnostics,
                            );
                        }
                        bindings.push(binding);
                    }
                }
                continue;
            }
            let requests = declarations[cursor].dependencies.clone();
            let request_source = declarations[cursor].common.source;
            for request in requests {
                requested_interfaces.push_back(request.token.service_type);
                if catalog
                    .get(&request.token)
                    .is_none_or(|&index| index >= explicit_count)
                    && let ProviderSource::Materialize(callback) = request.provider_source
                {
                    materialize(
                        request.token.service_type,
                        callback,
                        request_source,
                        &mut materialized,
                        &mut declarations,
                        &mut catalog,
                        explicit_count,
                        &mut diagnostics,
                    );
                }
            }
            cursor += 1;
        }

        // IDs are assigned only after closure, independently of linkme / queue arrival order.
        sort_declarations(&mut declarations);
        let mut routes: HashMap<_, _> = declarations
            .iter()
            .enumerate()
            .map(|(provider, declaration)| {
                (
                    declaration.identifier.clone(),
                    RootRoute {
                        provider,
                        projection: None,
                    },
                )
            })
            .collect();
        let mut by_type: HashMap<ServiceType, Vec<ProviderId>> = HashMap::new();
        for (provider, declaration) in declarations.iter().enumerate() {
            by_type
                .entry(declaration.identifier.service_type)
                .or_default()
                .push(provider);
        }

        bindings.sort_by_key(binding_order);
        let mut binding_index = HashMap::new();
        let mut candidates: HashMap<ServiceIdentifier, Vec<(ProviderId, TraitBinding)>> =
            HashMap::new();
        for binding in bindings {
            if let Some(original) =
                binding_index.insert((binding.trait_type, binding.concrete_type), binding)
            {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::DuplicateBinding,
                        format!(
                            "重复 trait binding {} -> {}：{} 与 {}",
                            binding.trait_type.name,
                            binding.concrete_type.name,
                            source(original.source),
                            source(binding.source),
                        ),
                    )
                    .at(&binding.trait_type.into(), original.source)
                    .at(&binding.concrete_type.into(), binding.source),
                );
                continue;
            }
            let Some(providers) = by_type.get(&binding.concrete_type) else {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::OrphanBinding,
                        format!(
                            "孤立 trait binding {} -> {}：concrete 类型没有 Provider（{}）",
                            binding.trait_type.name,
                            binding.concrete_type.name,
                            source(binding.source),
                        ),
                    )
                    .at(&binding.trait_type.into(), binding.source)
                    .at(&binding.concrete_type.into(), binding.source),
                );
                continue;
            };
            for &provider in providers {
                let identifier = ServiceIdentifier::new(
                    declarations[provider].identifier.service_key.clone(),
                    binding.trait_type,
                );
                candidates
                    .entry(identifier)
                    .or_default()
                    .push((provider, binding));
            }
        }

        let mut candidate_groups: Vec<_> = candidates.into_iter().collect();
        candidate_groups.sort_by(|(left, _), (right, _)| compare_tokens(left, right));
        for (identifier, candidates) in candidate_groups {
            let primaries: Vec<_> = candidates
                .iter()
                .filter(|(provider, _)| declarations[*provider].common.primary)
                .collect();
            let chosen = if candidates.len() == 1 {
                Some(&candidates[0])
            } else if primaries.len() == 1 {
                Some(primaries[0])
            } else {
                None
            };
            if let Some(&(provider, binding)) = chosen {
                if routes
                    .insert(
                        identifier.clone(),
                        RootRoute {
                            provider,
                            projection: Some(binding.prepare_required),
                        },
                    )
                    .is_some()
                {
                    diagnostics.push(
                        GraphDiagnostic::new(
                            Kind::InvalidMetadata,
                            format!("trait 路由与 concrete 注册冲突：{}", token(&identifier)),
                        )
                        .at(&identifier, binding.source),
                    );
                }
            } else {
                let mut diagnostic = GraphDiagnostic::new(
                    Kind::AmbiguousTrait,
                    format!(
                        "trait 候选不唯一 {}（{} 个 primary）：{}",
                        token(&identifier),
                        primaries.len(),
                        candidates
                            .iter()
                            .map(|(provider, _)| {
                                format!(
                                    "{}（{}）",
                                    token(&declarations[*provider].identifier),
                                    source(declarations[*provider].common.source)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("；"),
                    ),
                );
                diagnostic.services.push(identifier);
                for (provider, binding) in &candidates {
                    diagnostic = diagnostic
                        .at(&declarations[*provider].identifier, binding.source)
                        .at(
                            &declarations[*provider].identifier,
                            declarations[*provider].common.source,
                        );
                }
                diagnostics.push(diagnostic);
            }
        }

        let provider_types: Vec<_> = declarations
            .iter()
            .map(|declaration| declaration.identifier.service_type)
            .collect();
        let mut nodes = Vec::with_capacity(declarations.len());
        for declaration in &mut declarations {
            declaration
                .dependencies
                .sort_by_key(|dependency| dependency.input_slot);
            let mut dependencies = Vec::with_capacity(declaration.dependencies.len());
            for (position, request) in declaration.dependencies.iter().enumerate() {
                let location = format!(
                    "{} 的 {}（{}）",
                    token(&declaration.identifier),
                    dependency_label(request),
                    source(declaration.common.source)
                );
                if request.input_slot.index() != position {
                    diagnostics.push(
                        GraphDiagnostic::new(
                            Kind::InvalidMetadata,
                            format!(
                                "输入槽位必须连续且唯一：{location} 声明 {:?}，期望 {position}",
                                request.input_slot,
                            ),
                        )
                        .at(&declaration.identifier, declaration.common.source),
                    );
                }
                if matches!(
                    (request.optional, request.delivery),
                    (true, Delivery::RequiresBinding)
                        | (false, Delivery::RequiresBindingOrAbsent(_))
                ) {
                    diagnostics.push(
                        GraphDiagnostic::new(
                            Kind::InvalidMetadata,
                            format!("optional 与输入交付形态不一致：{location}"),
                        )
                        .at(&declaration.identifier, declaration.common.source),
                    );
                }
                if !matches!(request.delivery, Delivery::Direct(_))
                    && matches!(request.provider_source, ProviderSource::Materialize(_))
                {
                    diagnostics.push(
                        GraphDiagnostic::new(
                            Kind::InvalidMetadata,
                            format!("trait 请求不能携带 concrete 物化回调：{location}"),
                        )
                        .at(&declaration.identifier, declaration.common.source),
                    );
                }

                let route = routes.get(&request.token);
                if route.is_none() && !request.optional {
                    diagnostics.push(
                        GraphDiagnostic::new(
                            Kind::MissingDependency,
                            format!("缺少必选依赖 {}：{location}", token(&request.token),),
                        )
                        .at(&declaration.identifier, declaration.common.source)
                        .at(&request.token, declaration.common.source),
                    );
                }
                let prepare = match request.delivery {
                    Delivery::Direct(prepare) => {
                        if route.is_some_and(|route| route.projection.is_some()) {
                            diagnostics.push(
                                GraphDiagnostic::new(
                                    Kind::InvalidMetadata,
                                    format!("concrete 输入不能使用 trait 路由：{location}"),
                                )
                                .at(&declaration.identifier, declaration.common.source),
                            );
                        }
                        Some(prepare)
                    }
                    Delivery::RequiresBinding | Delivery::RequiresBindingOrAbsent(_) => {
                        if let Some(route) = route {
                            let concrete_type = provider_types[route.provider];
                            let binding =
                                binding_index.get(&(request.token.service_type, concrete_type));
                            match binding {
                                Some(binding) if route.projection.is_some() => {
                                    Some(if request.optional {
                                        binding.prepare_optional
                                    } else {
                                        binding.prepare_required
                                    })
                                }
                                _ => {
                                    diagnostics.push(
                                        GraphDiagnostic::new(
                                            Kind::InvalidMetadata,
                                            format!("trait 输入缺少类型化投影：{location}"),
                                        )
                                        .at(&declaration.identifier, declaration.common.source),
                                    );
                                    None
                                }
                            }
                        } else if let Delivery::RequiresBindingOrAbsent(absent) = request.delivery {
                            Some(absent)
                        } else {
                            None
                        }
                    }
                };
                if let Some(prepare) = prepare {
                    dependencies.push(CompiledDependency {
                        slot: request.input_slot,
                        requested: request.token.clone(),
                        optional: request.optional,
                        target: route.map(|route| route.provider),
                        prepare,
                        label: request.label,
                    });
                }
            }
            nodes.push(CompiledNode {
                identifier: declaration.identifier.clone(),
                common: declaration.common,
                dependencies,
                constructor: declaration.constructor,
                requires_scope: declaration.common.lifetime == ServiceLifetime::Scoped,
            });
        }

        let Topology {
            order: topological_order,
            dependencies: adjacency,
            dependents,
        } = topological_order(&nodes);
        if topological_order.len() != nodes.len() {
            diagnostics.extend(cycle_diagnostics(&nodes, &adjacency));
        } else {
            let mut scope_witness = vec![None; nodes.len()];
            for &provider in &topological_order {
                if let Some(target) = nodes[provider]
                    .dependencies
                    .iter()
                    .filter_map(|dependency| dependency.target)
                    .find(|&target| nodes[target].requires_scope)
                {
                    nodes[provider].requires_scope = true;
                    scope_witness[provider] = Some(target);
                }
            }
            for (provider, node) in nodes.iter().enumerate() {
                if node.common.lifetime == ServiceLifetime::Singleton && node.requires_scope {
                    let mut path = vec![provider];
                    let mut current = provider;
                    while nodes[current].common.lifetime != ServiceLifetime::Scoped {
                        let Some(next) = scope_witness[current] else {
                            break;
                        };
                        path.push(next);
                        current = next;
                    }
                    let mut diagnostic = GraphDiagnostic::new(
                        Kind::ScopeRequired,
                        format!(
                            "Singleton 的激活依赖需要 Scope：{}",
                            describe_path(&nodes, &path),
                        ),
                    );
                    for &provider in &path {
                        diagnostic = diagnostic
                            .at(&nodes[provider].identifier, nodes[provider].common.source);
                    }
                    diagnostics.push(diagnostic);
                }
            }
        }
        if !diagnostics.is_empty() {
            diagnostics.sort_by(|left, right| left.message.cmp(&right.message));
            diagnostics.dedup();
            return Err(GraphError { diagnostics });
        }
        Ok(ValidatedGraph {
            nodes,
            routes,
            topological_order,
            dependents,
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn materialize(
    expected: ServiceType,
    callback: ClosedProviderCallback,
    request_source: ServiceSource,
    materialized: &mut HashSet<(ServiceType, usize)>,
    declarations: &mut Vec<Declaration>,
    catalog: &mut HashMap<ServiceIdentifier, usize>,
    explicit_count: usize,
    diagnostics: &mut Vec<GraphDiagnostic>,
) {
    // Addresses are only an invocation cache, never evidence that two descriptors are
    // semantically equal. Equivalent callbacks may have different codegen addresses.
    if !materialized.insert((expected, callback as usize)) {
        return;
    }
    let declaration = Declaration::from(callback());
    if declaration.identifier.service_type != expected {
        diagnostics.push(
            GraphDiagnostic::new(
                Kind::MaterializationMismatch,
                format!(
                    "闭合 Provider 回调返回类型不匹配：请求 {}，实际 {}（{}）",
                    expected.name,
                    declaration.identifier.service_type.name,
                    source(request_source),
                ),
            )
            .at(&expected.into(), request_source)
            .at(&declaration.identifier, declaration.common.source),
        );
        return;
    }
    // The callback owns its key. An explicit registration wins only for the exact token.
    if let Some(&existing) = catalog.get(&declaration.identifier) {
        if existing >= explicit_count && !same_metadata(&declarations[existing], &declaration) {
            diagnostics.push(
                GraphDiagnostic::new(
                    Kind::InvalidMetadata,
                    format!(
                        "闭合 Provider 回调声明冲突：{}；同一 token 的可观察元数据不一致",
                        token(&declaration.identifier),
                    ),
                )
                .at(
                    &declarations[existing].identifier,
                    declarations[existing].common.source,
                )
                .at(&declaration.identifier, declaration.common.source),
            );
        }
        return;
    }
    catalog.insert(declaration.identifier.clone(), declarations.len());
    declarations.push(declaration);
}

/// Adapter bodies cannot be statically inspected. Their typed ABI is the contract; every
/// observable descriptor property must nevertheless agree when callbacks share a token.
fn same_metadata(left: &Declaration, right: &Declaration) -> bool {
    let constructor_kind = |constructor: Constructor| match constructor {
        Constructor::Class(_) => 0,
        Constructor::Factory(crate::registration::provider::FactoryInvoker::Sync(_)) => 1,
        Constructor::Factory(crate::registration::provider::FactoryInvoker::Async(_)) => 2,
    };
    left.identifier == right.identifier
        && left.common.lifetime == right.common.lifetime
        && left.common.primary == right.common.primary
        && left.common.source == right.common.source
        && left.common.cleanup.is_some() == right.common.cleanup.is_some()
        && constructor_kind(left.constructor) == constructor_kind(right.constructor)
        && left.dependencies.len() == right.dependencies.len()
        && left
            .dependencies
            .iter()
            .zip(&right.dependencies)
            .all(|(left, right)| {
                left.token == right.token
                    && left.input_slot == right.input_slot
                    && left.declaration_position == right.declaration_position
                    && left.optional == right.optional
                    && left.label == right.label
                    && std::mem::discriminant(&left.delivery)
                        == std::mem::discriminant(&right.delivery)
                    && std::mem::discriminant(&left.provider_source)
                        == std::mem::discriminant(&right.provider_source)
            })
}

fn binding_order(binding: &TraitBinding) -> (&'static str, &'static str, ServiceSource) {
    (
        binding.trait_type.name,
        binding.concrete_type.name,
        binding.source,
    )
}

fn sort_declarations(declarations: &mut [Declaration]) {
    declarations.sort_by(|left, right| {
        compare_tokens(&left.identifier, &right.identifier)
            .then(left.common.source.cmp(&right.common.source))
    });
}

fn compare_tokens(left: &ServiceIdentifier, right: &ServiceIdentifier) -> std::cmp::Ordering {
    left.service_type
        .name
        .cmp(right.service_type.name)
        .then(left.service_key.cmp(&right.service_key))
        .then(left.service_type.type_id.cmp(&right.service_type.type_id))
}

struct Topology {
    order: Vec<ProviderId>,
    dependencies: Vec<Vec<ProviderId>>,
    dependents: Vec<Vec<ProviderId>>,
}

fn topological_order(nodes: &[CompiledNode]) -> Topology {
    let adjacency: Vec<Vec<_>> = nodes
        .iter()
        .map(|node| {
            node.dependencies
                .iter()
                .filter_map(|dependency| dependency.target)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .collect();
    let mut consumers = vec![Vec::new(); nodes.len()];
    let mut remaining: Vec<_> = adjacency.iter().map(Vec::len).collect();
    for (provider, dependencies) in adjacency.iter().enumerate() {
        for &dependency in dependencies {
            consumers[dependency].push(provider);
        }
    }
    let mut ready: VecDeque<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(provider, &count)| (count == 0).then_some(provider))
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(provider) = ready.pop_front() {
        order.push(provider);
        for &consumer in &consumers[provider] {
            remaining[consumer] -= 1;
            if remaining[consumer] == 0 {
                ready.push_back(consumer);
            }
        }
    }
    Topology {
        order,
        dependencies: adjacency,
        dependents: consumers,
    }
}

fn cycle_diagnostics(
    nodes: &[CompiledNode],
    adjacency: &[Vec<ProviderId>],
) -> Vec<GraphDiagnostic> {
    let mut colors = vec![0_u8; nodes.len()];
    let mut positions = vec![0; nodes.len()];
    let mut diagnostics = Vec::new();
    for root in 0..nodes.len() {
        if colors[root] != 0 {
            continue;
        }
        colors[root] = 1;
        let mut stack = vec![(root, 0)];
        positions[root] = 0;
        while let Some((provider, next)) = stack.last_mut() {
            if *next == adjacency[*provider].len() {
                colors[*provider] = 2;
                stack.pop();
                continue;
            }
            let target = adjacency[*provider][*next];
            *next += 1;
            match colors[target] {
                0 => {
                    colors[target] = 1;
                    positions[target] = stack.len();
                    stack.push((target, 0));
                }
                1 => {
                    let mut cycle: Vec<_> = stack[positions[target]..]
                        .iter()
                        .map(|(provider, _)| *provider)
                        .collect();
                    cycle.push(target);
                    let mut diagnostic = GraphDiagnostic::new(
                        Kind::Cycle,
                        format!("循环依赖：{}", describe_path(nodes, &cycle)),
                    );
                    for &provider in &cycle {
                        diagnostic = diagnostic
                            .at(&nodes[provider].identifier, nodes[provider].common.source);
                    }
                    diagnostics.push(diagnostic);
                }
                _ => {}
            }
        }
    }
    diagnostics
}

fn describe_path(nodes: &[CompiledNode], path: &[ProviderId]) -> String {
    path.iter()
        .enumerate()
        .map(|(index, &provider)| {
            let node = &nodes[provider];
            let edge = path.get(index + 1).and_then(|next| {
                node.dependencies
                    .iter()
                    .find(|dependency| dependency.target == Some(*next))
            });
            let suffix = edge
                .map(|edge| match edge.label {
                    Some(label) => format!(".{label}"),
                    None => format!("[slot {}]", edge.slot.index()),
                })
                .unwrap_or_default();
            format!(
                "{}{}（{}）",
                token(&node.identifier),
                suffix,
                source(node.common.source)
            )
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}

fn dependency_label(request: &DependencyRequest) -> String {
    request
        .label
        .map(|label| format!("字段/参数 `{label}`"))
        .unwrap_or_else(|| format!("声明位置 {}", request.declaration_position))
}

fn token(identifier: &ServiceIdentifier) -> String {
    format!(
        "{} [key={:?}]",
        identifier.service_type.name, identifier.service_key
    )
}

fn source(source: ServiceSource) -> String {
    format!("{}:{}:{}", source.file, source.line, source.column)
}
