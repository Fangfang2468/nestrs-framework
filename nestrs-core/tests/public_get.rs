//! Black-box coverage for the intentionally narrow typed lookup API.
//!
//! Provider registration below uses only the macro-facing private ABI. Assertions exercise the
//! public `ServiceProvider` and `scope` APIs, so the test does not depend on Arena layout or
//! compiled-graph internals.

use nestrs_core::{
    __private::{
        ActivationError, ClassProvider, ConstructionContext, Delivery, DependencyRequest,
        ErasedService, Injectable, InputPosition, Lifetime, Provider, ProviderCommon,
        ProviderSource, ServiceIdentifier, ServiceSource, ServiceType, prepare_required,
    },
    ServiceProvider,
    scope::{ScopeEnd, ScopeLayer, ScopeProvider},
};

type Inject<T> = nestrs_core::__private::Inject<T>;

struct RootTransient {
    label: &'static str,
}

struct RootService {
    transient: Inject<RootTransient>,
}

struct MissingService;

struct ScopeApp {
    label: &'static str,
}

struct ScopeRequest {
    app: Inject<ScopeApp>,
    local: Inject<ScopeRequestLocal>,
}

struct ScopeRequestLocal {
    label: &'static str,
}

struct ScopeTransaction {
    app: Inject<ScopeApp>,
    request: Inject<ScopeRequest>,
    transient: Inject<ScopeTransient>,
}

struct ScopeTransient {
    label: &'static str,
}

type AccessScopeChain = ScopeLayer<ScopeRequest, ScopeLayer<ScopeTransaction, ScopeEnd>>;

fn identifier<T>() -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::from(ServiceType::create::<T>())
}

fn common(lifetime: Lifetime, line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime,
        primary: false,
        source: ServiceSource::new("tests/public_get.rs", line, 1),
        cleanup: None,
    }
}

fn construct_root_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(RootTransient { label: "root" }))
}

fn construct_root_service(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(RootService {
        transient: context.take::<RootTransient>(InputPosition(0))?,
    }))
}

fn construct_scope_app(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeApp { label: "app" }))
}

fn construct_scope_request_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeRequestLocal { label: "request" }))
}

fn construct_scope_request(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeRequest {
        app: context.take::<ScopeApp>(InputPosition(0))?,
        local: context.take::<ScopeRequestLocal>(InputPosition(1))?,
    }))
}

fn construct_scope_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeTransient {
        label: "transaction transient",
    }))
}

fn construct_scope_transaction(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeTransaction {
        app: context.take::<ScopeApp>(InputPosition(0))?,
        request: context.take::<ScopeRequest>(InputPosition(1))?,
        transient: context.take::<ScopeTransient>(InputPosition(2))?,
    }))
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn root_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<RootTransient>(),
        common: common(Lifetime::Transient, 95),
        dependencies: Vec::new(),
        constructor: construct_root_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn root_service_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<RootService>(),
        common: common(Lifetime::Singleton, 108),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<RootTransient>(),
            optional: false,
            label: Some("transient"),
            delivery: Delivery::Direct(prepare_required::<RootTransient>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_root_service,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeApp>(),
        common: common(Lifetime::Singleton, 128),
        dependencies: Vec::new(),
        constructor: construct_scope_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_request_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeRequestLocal>(),
        common: common(Lifetime::Scoped, 141),
        dependencies: Vec::new(),
        constructor: construct_scope_request_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeRequest>(),
        common: common(Lifetime::Scoped, 154),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopeApp>(),
                optional: false,
                label: Some("app"),
                delivery: Delivery::Direct(prepare_required::<ScopeApp>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ScopeRequestLocal>(),
                optional: false,
                label: Some("local"),
                delivery: Delivery::Direct(prepare_required::<ScopeRequestLocal>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_scope_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeTransient>(),
        common: common(Lifetime::Transient, 189),
        dependencies: Vec::new(),
        constructor: construct_scope_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_transaction_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeTransaction>(),
        common: common(Lifetime::Scoped, 202),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopeApp>(),
                optional: false,
                label: Some("app"),
                delivery: Delivery::Direct(prepare_required::<ScopeApp>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ScopeRequest>(),
                optional: false,
                label: Some("request"),
                delivery: Delivery::Direct(prepare_required::<ScopeRequest>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 2,
                input_position: InputPosition(2),
                token: identifier::<ScopeTransient>(),
                optional: false,
                label: Some("transient"),
                delivery: Delivery::Direct(prepare_required::<ScopeTransient>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_scope_transaction,
    })
}

#[test]
fn service_provider_get_exposes_singletons_but_not_transient_instances() {
    let provider = ServiceProvider::<RootService>::build().expect("root should build");

    let root = provider
        .get::<RootService>()
        .expect("the singleton root should be queryable by its concrete type");
    assert_eq!(root.transient.label, "root");
    assert!(std::ptr::eq(root, provider.root()));
    assert!(provider.get::<RootTransient>().is_none());
    assert!(provider.get::<MissingService>().is_none());
}

#[test]
fn scope_provider_and_scope_get_follow_visibility_and_lifetime_boundaries() {
    let provider = ScopeProvider::<ScopeApp, AccessScopeChain>::build()
        .expect("scope provider should build its singleton closure");

    assert_eq!(
        provider
            .get::<ScopeApp>()
            .expect("ScopeProvider should expose singleton services")
            .label,
        "app"
    );
    assert!(provider.get::<ScopeRequest>().is_none());
    assert!(provider.get::<ScopeRequestLocal>().is_none());
    assert!(provider.get::<ScopeTransaction>().is_none());
    assert!(provider.get::<ScopeTransient>().is_none());

    let request = provider
        .create_scope()
        .expect("the statically declared request scope should build");
    assert_eq!(
        request
            .get::<ScopeApp>()
            .expect("Scope should read parent singletons")
            .label,
        "app"
    );
    let request_root = request
        .get::<ScopeRequest>()
        .expect("Scope should expose its current scoped root");
    assert_eq!(request_root.app.label, "app");
    assert_eq!(request_root.local.label, "request");
    assert_eq!(
        request
            .get::<ScopeRequestLocal>()
            .expect("Scope should expose current scoped dependencies")
            .label,
        "request"
    );
    assert!(request.get::<ScopeTransaction>().is_none());
    assert!(request.get::<ScopeTransient>().is_none());

    let transaction = request
        .create_child_scope()
        .expect("the statically declared child scope should build");
    assert_eq!(
        transaction
            .get::<ScopeApp>()
            .expect("child Scope should read parent singletons")
            .label,
        "app"
    );
    assert_eq!(
        transaction
            .get::<ScopeRequest>()
            .expect("child Scope should read ancestor scoped services")
            .local
            .label,
        "request"
    );
    assert_eq!(
        transaction
            .get::<ScopeRequestLocal>()
            .expect("child Scope should read ancestor scoped dependencies")
            .label,
        "request"
    );
    let transaction_root = transaction
        .get::<ScopeTransaction>()
        .expect("child Scope should expose its own scoped root");
    assert_eq!(transaction_root.app.label, "app");
    assert_eq!(transaction_root.request.local.label, "request");
    assert_eq!(transaction_root.transient.label, "transaction transient");
    assert!(transaction.get::<ScopeTransient>().is_none());
}
