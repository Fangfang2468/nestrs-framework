use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use nestrs_core::{
    __private::{
        ActivationError, ArenaServiceRef, BoundKeyPolicy, ClassProvider, ConstructionContext,
        Delivery, DependencyRequest, ErasedService, Injectable, InputPosition, Lifetime, Provider,
        ProviderCommon, ProviderSource, REFLECTED_BINDINGS, REFLECTED_PROVIDERS, ServiceIdentifier,
        ServiceKey, ServiceSource, ServiceType, TraitBinding, prepare_bound_optional,
        prepare_bound_required, prepare_optional, prepare_required,
    },
    scope::{ScopeEnd, ScopeLayer, ScopeProvider},
};

type Inject<T> = nestrs_core::__private::Inject<T>;

struct KeyedEdgeApp;

trait AncestorKeyedPort: Send + Sync {
    fn instance(&self) -> usize;
}

struct AncestorKeyedAdapter {
    instance: usize,
}

impl AncestorKeyedPort for AncestorKeyedAdapter {
    fn instance(&self) -> usize {
        self.instance
    }
}

struct KeyedEdgeRequestRoot {
    port: Inject<dyn AncestorKeyedPort>,
}

struct MissingChildOptional;

struct KeyedEdgeTransactionRoot {
    inherited_port: Inject<dyn AncestorKeyedPort>,
    missing: Option<Inject<MissingChildOptional>>,
}

struct TransientEdgeApp;

struct TransientEdgeRequestRoot {
    instance: usize,
}

struct ScopedChildTransient {
    instance: usize,
    request: Inject<TransientEdgeRequestRoot>,
}

struct TransientEdgeTransactionRoot {
    first: Inject<ScopedChildTransient>,
    second: Inject<ScopedChildTransient>,
}

type KeyedEdgeChain =
    ScopeLayer<KeyedEdgeRequestRoot, ScopeLayer<KeyedEdgeTransactionRoot, ScopeEnd>>;
type TransientEdgeChain =
    ScopeLayer<TransientEdgeRequestRoot, ScopeLayer<TransientEdgeTransactionRoot, ScopeEnd>>;

static TEST_LOCK: Mutex<()> = Mutex::new(());
static KEYED_ADAPTER_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static TRANSIENT_REQUEST_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SCOPED_CHILD_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

fn identifier<T>() -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::from(ServiceType::create::<T>())
}

fn keyed_identifier<T>(key: &'static str) -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::new(Some(ServiceKey::Named(key)), ServiceType::create::<T>())
}

fn source(line: u32) -> ServiceSource {
    ServiceSource::new("tests/scope_chain_edges.rs", line, 1)
}

fn common(lifetime: Lifetime, line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime,
        primary: false,
        source: source(line),
        cleanup: None,
    }
}

fn singleton_common(line: u32) -> ProviderCommon {
    common(Lifetime::Singleton, line)
}

fn scoped_common(line: u32) -> ProviderCommon {
    common(Lifetime::Scoped, line)
}

fn transient_common(line: u32) -> ProviderCommon {
    common(Lifetime::Transient, line)
}

fn dependency<T>(position: usize) -> DependencyRequest
where
    T: Injectable,
{
    DependencyRequest {
        declaration_position: position,
        input_position: InputPosition(position),
        token: identifier::<T>(),
        optional: false,
        label: None,
        delivery: Delivery::Direct(prepare_required::<T>),
        provider_source: ProviderSource::Registered,
    }
}

fn optional<T>(position: usize) -> DependencyRequest
where
    T: Injectable,
{
    DependencyRequest {
        declaration_position: position,
        input_position: InputPosition(position),
        token: identifier::<T>(),
        optional: true,
        label: None,
        delivery: Delivery::Direct(prepare_optional::<T>),
        provider_source: ProviderSource::Registered,
    }
}

fn keyed_trait_dependency<T>(position: usize, key: &'static str) -> DependencyRequest
where
    T: Injectable + ?Sized,
{
    DependencyRequest {
        declaration_position: position,
        input_position: InputPosition(position),
        token: keyed_identifier::<T>(key),
        optional: false,
        label: Some("ancestor keyed trait port"),
        delivery: Delivery::RequiresBinding,
        provider_source: ProviderSource::Registered,
    }
}

fn construct_keyed_edge_app(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedEdgeApp))
}

fn construct_ancestor_keyed_adapter(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AncestorKeyedAdapter {
        instance: KEYED_ADAPTER_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_keyed_edge_request(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedEdgeRequestRoot {
        port: context.take::<dyn AncestorKeyedPort>(InputPosition(0))?,
    }))
}

fn construct_keyed_edge_transaction(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedEdgeTransactionRoot {
        inherited_port: context.take::<dyn AncestorKeyedPort>(InputPosition(0))?,
        missing: context.take_optional::<MissingChildOptional>(InputPosition(1))?,
    }))
}

fn construct_transient_edge_app(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(TransientEdgeApp))
}

fn construct_transient_edge_request(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(TransientEdgeRequestRoot {
        instance: TRANSIENT_REQUEST_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_scoped_child_transient(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedChildTransient {
        instance: SCOPED_CHILD_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
        request: context.take::<TransientEdgeRequestRoot>(InputPosition(0))?,
    }))
}

fn construct_transient_edge_transaction(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(TransientEdgeTransactionRoot {
        first: context.take::<ScopedChildTransient>(InputPosition(0))?,
        second: context.take::<ScopedChildTransient>(InputPosition(1))?,
    }))
}

fn project_ancestor_keyed_adapter(
    value: &AncestorKeyedAdapter,
) -> &(dyn AncestorKeyedPort + 'static) {
    value
}

fn prepare_ancestor_keyed_required(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_required::<AncestorKeyedAdapter, dyn AncestorKeyedPort>(
        context,
        position,
        input,
        project_ancestor_keyed_adapter,
    )
}

fn prepare_ancestor_keyed_optional(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_optional::<AncestorKeyedAdapter, dyn AncestorKeyedPort>(
        context,
        position,
        input,
        project_ancestor_keyed_adapter,
    )
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_edge_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<KeyedEdgeApp>(),
        common: singleton_common(100),
        dependencies: Vec::new(),
        constructor: construct_keyed_edge_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn ancestor_keyed_adapter_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: keyed_identifier::<AncestorKeyedAdapter>("ancestor-keyed-port"),
        common: scoped_common(110),
        dependencies: Vec::new(),
        constructor: construct_ancestor_keyed_adapter,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_edge_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<KeyedEdgeRequestRoot>(),
        common: scoped_common(120),
        dependencies: vec![keyed_trait_dependency::<dyn AncestorKeyedPort>(
            0,
            "ancestor-keyed-port",
        )],
        constructor: construct_keyed_edge_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_edge_transaction_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<KeyedEdgeTransactionRoot>(),
        common: scoped_common(130),
        dependencies: vec![
            keyed_trait_dependency::<dyn AncestorKeyedPort>(0, "ancestor-keyed-port"),
            optional::<MissingChildOptional>(1),
        ],
        constructor: construct_keyed_edge_transaction,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_BINDINGS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn ancestor_keyed_port_binding() -> TraitBinding {
    TraitBinding {
        trait_type: ServiceType::create::<dyn AncestorKeyedPort>(),
        concrete_type: ServiceType::create::<AncestorKeyedAdapter>(),
        key_policy: BoundKeyPolicy::InheritRequestedKey,
        prepare_required: prepare_ancestor_keyed_required,
        prepare_optional: prepare_ancestor_keyed_optional,
        source: source(140),
    }
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn transient_edge_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<TransientEdgeApp>(),
        common: singleton_common(200),
        dependencies: Vec::new(),
        constructor: construct_transient_edge_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn transient_edge_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<TransientEdgeRequestRoot>(),
        common: scoped_common(210),
        dependencies: Vec::new(),
        constructor: construct_transient_edge_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_child_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedChildTransient>(),
        common: transient_common(220),
        dependencies: vec![dependency::<TransientEdgeRequestRoot>(0)],
        constructor: construct_scoped_child_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn transient_edge_transaction_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<TransientEdgeTransactionRoot>(),
        common: scoped_common(230),
        dependencies: vec![
            dependency::<ScopedChildTransient>(0),
            dependency::<ScopedChildTransient>(1),
        ],
        constructor: construct_transient_edge_transaction,
    })
}

#[test]
fn child_scope_reads_ancestor_keyed_trait_binding_and_keeps_missing_optional_absent() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    KEYED_ADAPTER_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ScopeProvider::<KeyedEdgeApp, KeyedEdgeChain>::build()
        .expect("the keyed trait binding and optional child input should compile");
    let first_request = provider
        .create_scope()
        .expect("the first request scope should activate");
    let first_transaction = first_request
        .create_child_scope()
        .expect("the child should read the ancestor keyed binding");
    let sibling_transaction = first_request
        .create_child_scope()
        .expect("a sibling child should reuse the request-owned binding");
    let second_request = provider
        .create_scope()
        .expect("a second request should own a new keyed adapter");
    let second_transaction = second_request
        .create_child_scope()
        .expect("the second child should read its own request ancestor");

    assert_eq!(first_request.root().port.instance(), 1);
    assert_eq!(first_transaction.root().inherited_port.instance(), 1);
    assert_eq!(sibling_transaction.root().inherited_port.instance(), 1);
    assert!(std::ptr::eq(
        &*first_request.root().port,
        &*first_transaction.root().inherited_port,
    ));
    assert!(std::ptr::eq(
        &*first_request.root().port,
        &*sibling_transaction.root().inherited_port,
    ));
    assert!(first_transaction.root().missing.is_none());
    assert!(sibling_transaction.root().missing.is_none());

    assert_eq!(second_request.root().port.instance(), 2);
    assert_eq!(second_transaction.root().inherited_port.instance(), 2);
    assert!(std::ptr::eq(
        &*second_request.root().port,
        &*second_transaction.root().inherited_port,
    ));
    assert!(!std::ptr::eq(
        &*first_request.root().port,
        &*second_request.root().port,
    ));
    assert!(second_transaction.root().missing.is_none());
    assert_eq!(KEYED_ADAPTER_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
}

#[test]
fn child_scoped_consumers_own_transient_occurrences_and_can_capture_ancestor_scoped() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    TRANSIENT_REQUEST_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    SCOPED_CHILD_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ScopeProvider::<TransientEdgeApp, TransientEdgeChain>::build()
        .expect("the scoped child transient graph should compile");
    let first_request = provider
        .create_scope()
        .expect("the first request scope should activate");
    let first_transaction = first_request
        .create_child_scope()
        .expect("the first transaction scope should activate");
    let sibling_transaction = first_request
        .create_child_scope()
        .expect("a sibling transaction scope should activate independently");
    let second_request = provider
        .create_scope()
        .expect("the second request scope should activate");
    let second_transaction = second_request
        .create_child_scope()
        .expect("the second request child scope should activate");

    assert!(std::ptr::eq(
        &*first_transaction.root().first.request,
        first_request.root(),
    ));
    assert!(std::ptr::eq(
        &*first_transaction.root().second.request,
        first_request.root(),
    ));
    assert!(std::ptr::eq(
        &*sibling_transaction.root().first.request,
        first_request.root(),
    ));
    assert!(std::ptr::eq(
        &*sibling_transaction.root().second.request,
        first_request.root(),
    ));
    assert!(std::ptr::eq(
        &*second_transaction.root().first.request,
        second_request.root(),
    ));
    assert!(std::ptr::eq(
        &*second_transaction.root().second.request,
        second_request.root(),
    ));
    assert_eq!(first_request.root().instance, 1);
    assert_eq!(second_request.root().instance, 2);
    assert!(!std::ptr::eq(first_request.root(), second_request.root()));

    assert_ne!(
        &*first_transaction.root().first as *const ScopedChildTransient,
        &*first_transaction.root().second as *const ScopedChildTransient,
        "each child consumer input must own a distinct transient occurrence",
    );
    assert_ne!(
        &*first_transaction.root().first as *const ScopedChildTransient,
        &*sibling_transaction.root().first as *const ScopedChildTransient,
        "sibling child scopes must not share a transient occurrence",
    );
    assert_ne!(
        &*sibling_transaction.root().second as *const ScopedChildTransient,
        &*second_transaction.root().first as *const ScopedChildTransient,
        "children below different request scopes must not share an occurrence",
    );
    assert_ne!(
        first_transaction.root().first.instance,
        first_transaction.root().second.instance,
    );
    assert_ne!(
        sibling_transaction.root().first.instance,
        sibling_transaction.root().second.instance,
    );
    assert_ne!(
        second_transaction.root().first.instance,
        second_transaction.root().second.instance,
    );
    assert_eq!(TRANSIENT_REQUEST_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
    assert_eq!(
        SCOPED_CHILD_TRANSIENT_CONSTRUCTIONS.load(Ordering::SeqCst),
        6
    );
}
