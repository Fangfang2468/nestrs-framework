use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::{
    ServiceKey, ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, prepare_bound_optional,
        prepare_bound_required, prepare_optional, prepare_optional_absent, prepare_required,
    },
    registration::{
        binding::{BoundKeyPolicy, TraitBinding},
        dependency::{Delivery, DependencyRequest, ProviderSource},
        provider::{ClassProvider, FactoryInvoker, FactoryProvider, Provider, ProviderCommon},
        root::RootDeclaration,
    },
    service::{ServiceSource, ServiceType},
};

struct Alpha;
struct Beta;
struct Gamma;
struct Consumer;
trait Audit: Send + Sync {}
impl Audit for Alpha {}
impl Audit for Beta {}

fn source() -> ServiceSource {
    ServiceSource::new("graph-test.rs", 10, 2)
}

fn common(lifetime: ServiceLifetime) -> ProviderCommon {
    ProviderCommon {
        lifetime,
        primary: false,
        source: source(),
        cleanup: None,
    }
}

fn must_not_construct(_: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    panic!("graph compilation must never construct services")
}

fn token<T: ?Sized + Send + Sync + 'static>(key: Option<ServiceKey>) -> ServiceIdentifier {
    ServiceIdentifier::new(key, ServiceType::create::<T>())
}

fn provider<T: Send + Sync + 'static>(
    key: Option<ServiceKey>,
    lifetime: ServiceLifetime,
    dependencies: Vec<DependencyRequest>,
) -> Provider {
    Provider::Class(ClassProvider {
        provide: token::<T>(key),
        common: common(lifetime),
        dependencies,
        constructor: must_not_construct,
    })
}

fn dependency<T: Send + Sync + 'static>(slot: usize, key: Option<ServiceKey>) -> DependencyRequest {
    DependencyRequest {
        declaration_position: slot,
        input_slot: InputSlot::new(slot),
        token: token::<T>(key),
        optional: false,
        label: Some("dependency"),
        delivery: Delivery::Direct(prepare_required::<T>),
        provider_source: ProviderSource::Registered,
    }
}

fn binding<T: Audit + 'static>() -> TraitBinding {
    TraitBinding {
        trait_type: ServiceType::create::<dyn Audit>(),
        concrete_type: ServiceType::create::<T>(),
        materialize: None,
        key_policy: BoundKeyPolicy::InheritRequestedKey,
        prepare_required: |slot, value| {
            prepare_bound_required::<T, dyn Audit>(slot, value, |value| value)
        },
        prepare_optional: |slot, value| {
            prepare_bound_optional::<T, dyn Audit>(slot, value, |value| value)
        },
        source: source(),
    }
}

fn trait_dependency(optional: bool, key: Option<ServiceKey>) -> DependencyRequest {
    DependencyRequest {
        declaration_position: 0,
        input_slot: InputSlot::new(0),
        token: token::<dyn Audit>(key),
        optional,
        label: Some("audit"),
        delivery: if optional {
            Delivery::RequiresBindingOrAbsent(prepare_optional_absent::<dyn Audit>)
        } else {
            Delivery::RequiresBinding
        },
        provider_source: ProviderSource::Registered,
    }
}

fn compile(providers: Vec<Provider>) -> Result<ValidatedGraph, GraphError> {
    GraphCompiler::compile_snapshot(providers, vec![], vec![])
}

fn audit_root() -> RootDeclaration {
    RootDeclaration {
        service_type: ServiceType::create::<dyn Audit>(),
        materialize: None,
        source: source(),
    }
}

#[test]
fn unused_automatic_capabilities_do_not_create_routes_or_materialize_blueprints() {
    let mut unused = binding::<Alpha>();
    unused.materialize = Some(|| panic!("an unused capability must not materialize a provider"));
    let graph = GraphCompiler::compile_with_automatic_bindings(
        vec![
            provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Beta>(None, ServiceLifetime::Singleton, vec![]),
        ],
        vec![],
        vec![],
        vec![unused, binding::<Beta>()],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert!(!graph.routes.contains_key(&token::<dyn Audit>(None)));
}

#[test]
fn sibling_automatic_capabilities_are_idempotent_and_deterministic() {
    let mut later = binding::<Alpha>();
    later.source = ServiceSource::new("z-sibling.rs", 2, 1);
    for automatic in [
        vec![later, binding::<Alpha>()],
        vec![binding::<Alpha>(), later],
    ] {
        let graph = GraphCompiler::compile_with_automatic_bindings(
            vec![provider::<Alpha>(None, ServiceLifetime::Singleton, vec![])],
            vec![],
            vec![audit_root(), audit_root()],
            automatic,
        )
        .unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(
            graph.routes[&token::<Alpha>(None)].provider,
            graph.routes[&token::<dyn Audit>(None)].provider,
        );
    }
}

#[test]
fn explicit_binding_wins_over_automatic_without_hiding_explicit_duplicates() {
    let mut ignored = binding::<Alpha>();
    ignored.materialize = Some(|| panic!("explicit projection must supersede automatic blueprint"));
    for count in [1, 2] {
        let result = GraphCompiler::compile_with_automatic_bindings(
            vec![provider::<Alpha>(None, ServiceLifetime::Singleton, vec![])],
            vec![binding::<Alpha>(); count],
            vec![audit_root()],
            vec![ignored, ignored],
        );
        if count == 1 {
            assert!(result.is_ok());
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("重复 trait binding")
            );
        }
    }
}

#[test]
fn automatic_capabilities_activate_from_registered_provider_dependencies() {
    let graph = GraphCompiler::compile_with_automatic_bindings(
        vec![
            provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Consumer>(
                None,
                ServiceLifetime::Singleton,
                vec![trait_dependency(false, None)],
            ),
        ],
        vec![],
        vec![],
        vec![binding::<Alpha>()],
    )
    .unwrap();
    let consumer = graph.routes[&token::<Consumer>(None)].provider;
    let alpha = graph.routes[&token::<Alpha>(None)].provider;
    assert_eq!(graph.nodes[consumer].dependencies[0].target, Some(alpha));
}

#[test]
fn automatic_materialization_closes_new_concrete_and_interface_dependencies() {
    trait Store: Send + Sync {}
    impl Store for Gamma {}
    let store = TraitBinding {
        trait_type: ServiceType::create::<dyn Store>(),
        concrete_type: ServiceType::create::<Gamma>(),
        materialize: Some(|| provider::<Gamma>(None, ServiceLifetime::Singleton, vec![])),
        key_policy: BoundKeyPolicy::InheritRequestedKey,
        prepare_required: |slot, value| {
            prepare_bound_required::<Gamma, dyn Store>(slot, value, |value| value)
        },
        prepare_optional: |slot, value| {
            prepare_bound_optional::<Gamma, dyn Store>(slot, value, |value| value)
        },
        source: source(),
    };
    let mut audit = binding::<Alpha>();
    audit.materialize = Some(|| {
        let mut request = trait_dependency(false, None);
        request.token = token::<dyn Store>(None);
        provider::<Alpha>(None, ServiceLifetime::Singleton, vec![request])
    });
    let graph = GraphCompiler::compile_with_automatic_bindings(
        vec![],
        vec![],
        vec![audit_root()],
        vec![store, audit],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    let alpha = graph.routes[&token::<Alpha>(None)].provider;
    let gamma = graph.routes[&token::<Gamma>(None)].provider;
    assert_eq!(graph.nodes[alpha].dependencies[0].target, Some(gamma));
    assert_eq!(graph.topological_order, vec![gamma, alpha]);
}

#[test]
fn optional_automatic_requests_cannot_hide_ambiguity_or_lifetime_errors() {
    let ambiguous = GraphCompiler::compile_with_automatic_bindings(
        vec![
            provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Beta>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Consumer>(
                None,
                ServiceLifetime::Singleton,
                vec![trait_dependency(true, None)],
            ),
        ],
        vec![],
        vec![],
        vec![binding::<Alpha>(), binding::<Beta>()],
    )
    .unwrap_err();
    assert!(ambiguous.to_string().contains("AmbiguousTrait"));
    let scoped = GraphCompiler::compile_with_automatic_bindings(
        vec![
            provider::<Alpha>(None, ServiceLifetime::Scoped, vec![]),
            provider::<Consumer>(
                None,
                ServiceLifetime::Singleton,
                vec![trait_dependency(true, None)],
            ),
        ],
        vec![],
        vec![],
        vec![binding::<Alpha>()],
    )
    .unwrap_err();
    assert!(scoped.to_string().contains("ScopeRequired"));
}

#[test]
fn automatic_capabilities_keep_keyed_routes_and_exact_explicit_provider_priority() {
    let named = Some(ServiceKey::Named("audit".into()));
    let mut audit = binding::<Alpha>();
    audit.materialize = Some(|| {
        provider::<Alpha>(
            Some(ServiceKey::Named("audit".into())),
            ServiceLifetime::Scoped,
            vec![dependency::<Gamma>(0, None)],
        )
    });
    let graph = GraphCompiler::compile_with_automatic_bindings(
        vec![provider::<Alpha>(
            named.clone(),
            ServiceLifetime::Singleton,
            vec![],
        )],
        vec![],
        vec![audit_root()],
        vec![audit],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert!(graph.routes.contains_key(&token::<dyn Audit>(named)));
    assert!(!graph.routes.contains_key(&token::<dyn Audit>(None)));
    assert_eq!(graph.nodes[0].common.lifetime, ServiceLifetime::Singleton);
}

#[test]
fn snapshot_order_is_deterministic_and_optional_absence_is_frozen() {
    let mut absent = dependency::<Gamma>(1, None);
    absent.optional = true;
    absent.delivery = Delivery::Direct(prepare_optional::<Gamma>);
    let providers = vec![
        provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]),
        provider::<Consumer>(
            None,
            ServiceLifetime::Singleton,
            vec![dependency::<Alpha>(0, None), absent],
        ),
    ];
    let forward = compile(providers.clone()).unwrap();
    let reverse = compile(providers.into_iter().rev().collect()).unwrap();
    assert_eq!(
        forward
            .nodes
            .iter()
            .map(|node| &node.identifier)
            .collect::<Vec<_>>(),
        reverse
            .nodes
            .iter()
            .map(|node| &node.identifier)
            .collect::<Vec<_>>()
    );
    assert_eq!(forward.topological_order, reverse.topological_order);
    let consumer = forward.routes[&token::<Consumer>(None)].provider;
    let alpha = forward.routes[&token::<Alpha>(None)].provider;
    assert_eq!(forward.nodes[consumer].dependencies[0].target, Some(alpha));
    assert_eq!(forward.nodes[consumer].dependencies[1].target, None);
    assert_eq!(forward.dependents[alpha], vec![consumer]);
    assert!(
        forward.topological_order.iter().position(|&id| id == alpha)
            < forward
                .topological_order
                .iter()
                .position(|&id| id == consumer)
    );
}

#[test]
fn all_declarations_are_checked_even_if_no_root_requests_them() {
    let error = compile(vec![provider::<Consumer>(
        None,
        ServiceLifetime::Singleton,
        vec![dependency::<Alpha>(0, None)],
    )])
    .unwrap_err()
    .to_string();
    assert!(error.contains("缺少必选依赖"));
    assert!(error.contains("字段/参数 `dependency`"));
    assert!(error.contains("graph-test.rs:10:2"));
}

#[test]
fn primary_never_hides_duplicate_concrete_registration() {
    let first = provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]);
    let mut second = first.clone();
    let Provider::Class(provider) = &mut second else {
        unreachable!()
    };
    provider.common.primary = true;
    assert!(
        compile(vec![first, second])
            .unwrap_err()
            .to_string()
            .contains("重复 Provider")
    );
}

#[test]
fn trait_routes_use_exact_keys_and_primary_is_local_to_each_candidate_set() {
    let mut primary = provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]);
    let Provider::Class(alpha) = &mut primary else {
        unreachable!()
    };
    alpha.common.primary = true;
    let graph = GraphCompiler::compile_snapshot(
        vec![
            primary,
            provider::<Beta>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Beta>(
                Some(ServiceKey::Named("special".into())),
                ServiceLifetime::Scoped,
                vec![],
            ),
            provider::<Consumer>(
                None,
                ServiceLifetime::Singleton,
                vec![trait_dependency(false, None)],
            ),
        ],
        vec![binding::<Alpha>(), binding::<Beta>()],
        vec![],
    )
    .unwrap();
    let route = graph.routes[&token::<dyn Audit>(None)];
    assert_eq!(graph.nodes[route.provider].identifier, token::<Alpha>(None));
    assert!(route.projection.is_some());
    let keyed = token::<dyn Audit>(Some(ServiceKey::Named("special".into())));
    assert_eq!(
        graph.nodes[graph.routes[&keyed].provider]
            .identifier
            .service_type,
        ServiceType::create::<Beta>()
    );
    let consumer = graph.routes[&token::<Consumer>(None)].provider;
    assert_eq!(
        graph.nodes[consumer].dependencies[0].target,
        Some(route.provider)
    );
}

#[test]
fn named_registration_never_satisfies_a_default_key_request() {
    let error = GraphCompiler::compile_snapshot(
        vec![
            provider::<Alpha>(
                Some(ServiceKey::Named("only".into())),
                ServiceLifetime::Singleton,
                vec![],
            ),
            provider::<Consumer>(
                None,
                ServiceLifetime::Singleton,
                vec![trait_dependency(false, None)],
            ),
        ],
        vec![binding::<Alpha>()],
        vec![],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("缺少必选依赖"));
    assert!(error.contains("key=None"));
}

#[test]
fn trait_ambiguity_is_rejected_even_when_not_consumed_or_only_optional() {
    for primary_count in [0, 2] {
        let mut providers = vec![
            provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Beta>(None, ServiceLifetime::Singleton, vec![]),
        ];
        for provider in providers.iter_mut().take(primary_count) {
            let Provider::Class(provider) = provider else {
                unreachable!()
            };
            provider.common.primary = true;
        }
        providers.push(provider::<Consumer>(
            None,
            ServiceLifetime::Singleton,
            vec![trait_dependency(true, None)],
        ));
        let error = GraphCompiler::compile_snapshot(
            providers,
            vec![binding::<Alpha>(), binding::<Beta>()],
            vec![],
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("trait 候选不唯一"));
        assert!(error.contains(&format!("{primary_count} 个 primary")));
    }
}

#[test]
fn absent_optional_trait_compiles_without_a_binding() {
    let graph = compile(vec![provider::<Consumer>(
        None,
        ServiceLifetime::Singleton,
        vec![trait_dependency(true, None)],
    )])
    .unwrap();
    assert_eq!(graph.nodes[0].dependencies[0].target, None);
    assert!((graph.nodes[0].dependencies[0].prepare)(InputSlot::new(0), None).is_ok());
}

#[test]
fn orphan_and_duplicate_bindings_are_errors() {
    let error = GraphCompiler::compile_snapshot(vec![], vec![binding::<Alpha>()], vec![])
        .unwrap_err()
        .to_string();
    assert!(error.contains("孤立 trait binding"));
    let error = GraphCompiler::compile_snapshot(
        vec![provider::<Alpha>(None, ServiceLifetime::Singleton, vec![])],
        vec![binding::<Alpha>(), binding::<Alpha>()],
        vec![],
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("重复 trait binding"));
}

struct GenericRoot;
struct GenericLeaf;
static ROOT_CALLBACKS: AtomicUsize = AtomicUsize::new(0);
static LEAF_CALLBACKS: AtomicUsize = AtomicUsize::new(0);

fn leaf_definition() -> Provider {
    LEAF_CALLBACKS.fetch_add(1, Ordering::SeqCst);
    provider::<GenericLeaf>(None, ServiceLifetime::Singleton, vec![])
}

fn root_definition() -> Provider {
    ROOT_CALLBACKS.fetch_add(1, Ordering::SeqCst);
    let mut leaf = dependency::<GenericLeaf>(0, None);
    leaf.provider_source = ProviderSource::Materialize(leaf_definition);
    let mut another_leaf = leaf.clone();
    another_leaf.declaration_position = 1;
    another_leaf.input_slot = InputSlot::new(1);
    provider::<GenericRoot>(None, ServiceLifetime::Singleton, vec![leaf, another_leaf])
}

fn root<T: Send + Sync + 'static>(callback: fn() -> Provider) -> RootDeclaration {
    RootDeclaration {
        service_type: ServiceType::create::<T>(),
        materialize: Some(callback),
        source: source(),
    }
}

#[test]
fn closed_roots_are_idempotent_and_materialization_closes_the_whole_graph() {
    ROOT_CALLBACKS.store(0, Ordering::SeqCst);
    LEAF_CALLBACKS.store(0, Ordering::SeqCst);
    let root = root::<GenericRoot>(root_definition);
    let graph = GraphCompiler::compile_snapshot(vec![], vec![], vec![root, root]).unwrap();
    assert_eq!(ROOT_CALLBACKS.load(Ordering::SeqCst), 1);
    assert_eq!(LEAF_CALLBACKS.load(Ordering::SeqCst), 1);
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph.routes.contains_key(&token::<GenericRoot>(None)));
    assert!(graph.routes.contains_key(&token::<GenericLeaf>(None)));
    let root_id = graph.routes[&token::<GenericRoot>(None)].provider;
    assert_eq!(graph.nodes[root_id].dependencies.len(), 2);
    assert_eq!(
        graph.nodes[root_id].dependencies[0].target,
        graph.nodes[root_id].dependencies[1].target
    );
}

#[test]
fn root_requirements_without_callbacks_do_not_create_missing_provider_errors() {
    let roots = vec![
        RootDeclaration {
            service_type: ServiceType::create::<Alpha>(),
            materialize: None,
            source: source(),
        },
        RootDeclaration {
            service_type: ServiceType::create::<dyn Audit>(),
            materialize: None,
            source: source(),
        },
    ];
    let graph = GraphCompiler::compile_snapshot(vec![], vec![], roots).unwrap();
    assert!(graph.nodes.is_empty());
    assert!(graph.routes.is_empty());
}

#[test]
fn a_root_without_callback_never_masks_a_later_materializable_query() {
    let absent = RootDeclaration {
        service_type: ServiceType::create::<Alpha>(),
        materialize: None,
        source: ServiceSource::new("graph-test.rs", 1, 1),
    };
    let defined = root::<Alpha>(keyed_alpha_definition);
    for roots in [
        vec![absent, defined, absent, defined],
        vec![defined, absent],
    ] {
        let graph = GraphCompiler::compile_snapshot(vec![], vec![], roots).unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert!(
            graph
                .routes
                .contains_key(&token::<Alpha>(Some(ServiceKey::Named("declared".into()))))
        );
    }
}

fn keyed_alpha_definition() -> Provider {
    provider::<Alpha>(
        Some(ServiceKey::Named("declared".into())),
        ServiceLifetime::Scoped,
        vec![],
    )
}

#[test]
fn roots_keep_callback_keys_and_only_exact_explicit_registration_wins() {
    let key = Some(ServiceKey::Named("declared".into()));
    let graph = GraphCompiler::compile_snapshot(
        vec![provider::<Alpha>(None, ServiceLifetime::Singleton, vec![])],
        vec![binding::<Alpha>()],
        vec![root::<Alpha>(keyed_alpha_definition)],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 2);
    assert!(graph.routes.contains_key(&token::<Alpha>(None)));
    assert!(graph.routes.contains_key(&token::<Alpha>(key.clone())));
    assert!(graph.routes.contains_key(&token::<dyn Audit>(key.clone())));

    let graph = GraphCompiler::compile_snapshot(
        vec![provider::<Alpha>(
            key.clone(),
            ServiceLifetime::Transient,
            vec![],
        )],
        vec![],
        vec![root::<Alpha>(keyed_alpha_definition)],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].common.lifetime, ServiceLifetime::Transient);
}

#[test]
fn root_materialization_happens_before_orphan_binding_validation() {
    let graph = GraphCompiler::compile_snapshot(
        vec![],
        vec![binding::<Alpha>()],
        vec![root::<Alpha>(keyed_alpha_definition)],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 1);
}

#[test]
fn a_binding_blueprint_closes_the_concrete_graph_without_a_concrete_query_root() {
    let mut generic_binding = binding::<Alpha>();
    generic_binding.materialize = Some(keyed_alpha_definition);
    let trait_only_query = RootDeclaration {
        service_type: ServiceType::create::<dyn Audit>(),
        materialize: None,
        source: source(),
    };
    let graph =
        GraphCompiler::compile_snapshot(vec![], vec![generic_binding], vec![trait_only_query])
            .unwrap();
    assert_eq!(graph.nodes.len(), 1);
    let key = Some(ServiceKey::Named("declared".into()));
    assert!(graph.routes.contains_key(&token::<dyn Audit>(key.clone())));
    assert!(!graph.routes.contains_key(&token::<dyn Audit>(None)));
    assert_eq!(graph.nodes[0].identifier, token::<Alpha>(key));
}

#[test]
fn binding_blueprints_reuse_explicit_exact_providers() {
    let mut generic_binding = binding::<Alpha>();
    generic_binding.materialize = Some(keyed_alpha_definition);
    let key = Some(ServiceKey::Named("declared".into()));
    let graph = GraphCompiler::compile_snapshot(
        vec![provider::<Alpha>(
            key.clone(),
            ServiceLifetime::Transient,
            vec![],
        )],
        vec![generic_binding],
        vec![],
    )
    .unwrap();
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].common.lifetime, ServiceLifetime::Transient);
    assert_eq!(graph.routes[&token::<dyn Audit>(key)].provider, 0);
}

#[test]
fn callback_type_mismatch_is_a_build_error() {
    let error =
        GraphCompiler::compile_snapshot(vec![], vec![], vec![root::<Beta>(keyed_alpha_definition)])
            .unwrap_err()
            .to_string();
    assert!(error.contains("回调返回类型不匹配"));
}

#[test]
fn dependency_materialization_does_not_rewrite_the_declared_key() {
    let mut request = dependency::<Alpha>(0, None);
    request.provider_source = ProviderSource::Materialize(keyed_alpha_definition);
    let error = compile(vec![provider::<Consumer>(
        None,
        ServiceLifetime::Scoped,
        vec![request],
    )])
    .unwrap_err()
    .to_string();
    assert!(error.contains("缺少必选依赖"));
    assert!(error.contains("key=None"));
}

fn first_key_definition() -> Provider {
    provider::<Alpha>(
        Some(ServiceKey::Indexed(1)),
        ServiceLifetime::Singleton,
        vec![],
    )
}

fn second_key_definition() -> Provider {
    provider::<Alpha>(
        Some(ServiceKey::Indexed(2)),
        ServiceLifetime::Singleton,
        vec![],
    )
}

fn plain_alpha_definition() -> Provider {
    provider::<Alpha>(None, ServiceLifetime::Singleton, vec![])
}

fn equivalent_alpha_definition() -> Provider {
    std::hint::black_box(());
    provider::<Alpha>(None, ServiceLifetime::Singleton, vec![])
}

fn conflicting_alpha_definition() -> Provider {
    provider::<Alpha>(None, ServiceLifetime::Transient, vec![])
}

#[test]
fn distinct_materialization_callbacks_can_describe_different_keys_of_the_same_type() {
    let mut first = dependency::<Alpha>(0, Some(ServiceKey::Indexed(1)));
    first.provider_source = ProviderSource::Materialize(first_key_definition);
    let mut second = dependency::<Alpha>(1, Some(ServiceKey::Indexed(2)));
    second.provider_source = ProviderSource::Materialize(second_key_definition);
    let graph = compile(vec![provider::<Consumer>(
        None,
        ServiceLifetime::Singleton,
        vec![first, second],
    )])
    .unwrap();
    assert_eq!(graph.nodes.len(), 3);
    assert!(
        graph
            .routes
            .contains_key(&token::<Alpha>(Some(ServiceKey::Indexed(1))))
    );
    assert!(
        graph
            .routes
            .contains_key(&token::<Alpha>(Some(ServiceKey::Indexed(2))))
    );
}

#[test]
fn different_callback_addresses_with_equal_metadata_are_idempotent_but_conflicts_fail() {
    for (callback, should_succeed) in [
        (equivalent_alpha_definition as fn() -> Provider, true),
        (conflicting_alpha_definition as fn() -> Provider, false),
    ] {
        let mut first = dependency::<Alpha>(0, None);
        first.provider_source = ProviderSource::Materialize(plain_alpha_definition);
        let mut second = dependency::<Alpha>(1, None);
        second.provider_source = ProviderSource::Materialize(callback);
        let result = compile(vec![provider::<Consumer>(
            None,
            ServiceLifetime::Singleton,
            vec![first, second],
        )]);
        if should_succeed {
            assert_eq!(result.unwrap().nodes.len(), 2);
        } else {
            assert!(result.unwrap_err().to_string().contains("回调声明冲突"));
        }
    }
}

#[test]
fn invalid_input_slots_and_delivery_contracts_are_rejected() {
    for slots in [[0, 0], [0, 2], [1, 2]] {
        let requests = slots
            .into_iter()
            .map(|slot| dependency::<Alpha>(slot, None))
            .collect();
        let error = compile(vec![
            provider::<Alpha>(None, ServiceLifetime::Singleton, vec![]),
            provider::<Consumer>(None, ServiceLifetime::Singleton, requests),
        ])
        .unwrap_err()
        .to_string();
        assert!(error.contains("输入槽位必须连续且唯一"));
    }
    let mut request = trait_dependency(true, None);
    request.delivery = Delivery::RequiresBinding;
    let error = compile(vec![provider::<Consumer>(
        None,
        ServiceLifetime::Singleton,
        vec![request],
    )])
    .unwrap_err()
    .to_string();
    assert!(error.contains("optional 与输入交付形态不一致"));
}

#[test]
fn cycles_include_optional_present_and_factory_edges_but_not_downstream_nodes() {
    let mut b_to_a = dependency::<Alpha>(0, None);
    b_to_a.optional = true;
    b_to_a.delivery = Delivery::Direct(prepare_optional::<Alpha>);
    let factory = Provider::Factory(FactoryProvider {
        provide: token::<Beta>(None),
        common: common(ServiceLifetime::Transient),
        dependencies: vec![b_to_a],
        invoker: FactoryInvoker::Sync(|_| panic!("not at build")),
    });
    let error = compile(vec![
        provider::<Alpha>(
            None,
            ServiceLifetime::Transient,
            vec![dependency::<Beta>(0, None)],
        ),
        factory,
        provider::<Consumer>(
            None,
            ServiceLifetime::Transient,
            vec![dependency::<Alpha>(0, None)],
        ),
    ])
    .unwrap_err();
    let diagnostic = error
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.kind == GraphDiagnosticKind::Cycle)
        .unwrap();
    assert_eq!(diagnostic.services.len(), 3);
    assert_eq!(diagnostic.sources.len(), 3);
    let cycle = &diagnostic.message;
    assert!(cycle.contains("Alpha"));
    assert!(cycle.contains("Beta"));
    assert!(cycle.contains(".dependency"));
    assert!(!cycle.contains("Consumer"));
}

#[test]
fn singleton_can_retain_transient_but_never_a_transitive_scope_dependency() {
    let graph = compile(vec![
        provider::<Alpha>(None, ServiceLifetime::Transient, vec![]),
        provider::<Consumer>(
            None,
            ServiceLifetime::Singleton,
            vec![dependency::<Alpha>(0, None)],
        ),
    ])
    .unwrap();
    assert!(graph.nodes.iter().all(|node| !node.requires_scope));

    let chain = vec![
        provider::<Alpha>(None, ServiceLifetime::Scoped, vec![]),
        provider::<Beta>(
            None,
            ServiceLifetime::Transient,
            vec![dependency::<Alpha>(0, None)],
        ),
        provider::<Consumer>(
            None,
            ServiceLifetime::Singleton,
            vec![dependency::<Beta>(0, None)],
        ),
    ];
    let error = compile(chain).unwrap_err().to_string();
    assert!(error.contains("Singleton 的激活依赖需要 Scope"));
    assert!(
        error.contains("Consumer.dependency") || error.contains("Consumer [key=None].dependency")
    );
    assert!(error.contains("Beta"));
    assert!(error.contains("Alpha"));
}

#[test]
fn transient_scope_requirement_is_precomputed_and_factory_borrows_cannot_bypass_it() {
    let providers = vec![
        provider::<Alpha>(None, ServiceLifetime::Scoped, vec![]),
        provider::<Beta>(
            None,
            ServiceLifetime::Transient,
            vec![dependency::<Alpha>(0, None)],
        ),
    ];
    let graph = compile(providers.clone()).unwrap();
    assert!(graph.nodes[graph.routes[&token::<Beta>(None)].provider].requires_scope);
    let mut providers = providers;
    providers.push(Provider::Factory(FactoryProvider {
        provide: token::<Consumer>(None),
        common: common(ServiceLifetime::Singleton),
        dependencies: vec![dependency::<Beta>(0, None)],
        invoker: FactoryInvoker::Sync(|_| panic!("not at build")),
    }));
    assert!(
        compile(providers)
            .unwrap_err()
            .to_string()
            .contains("需要 Scope")
    );
}

#[test]
fn twenty_thousand_node_graph_compiles_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            const COUNT: usize = 20_000;
            let providers = (0..COUNT)
                .map(|index| {
                    let dependencies = if index == 0 {
                        vec![]
                    } else {
                        vec![dependency::<Alpha>(0, Some(ServiceKey::Indexed(index - 1)))]
                    };
                    provider::<Alpha>(
                        Some(ServiceKey::Indexed(index)),
                        ServiceLifetime::Transient,
                        dependencies,
                    )
                })
                .collect();
            let graph = compile(providers).unwrap();
            assert_eq!(graph.topological_order.len(), COUNT);
            assert!(graph.nodes.iter().all(|node| !node.requires_scope));
        })
        .unwrap()
        .join()
        .unwrap();
}
