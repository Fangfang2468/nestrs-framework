use std::{
    future::Future,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use nestrs_core::{
    __private::{
        ActivationError, ClassProvider, CleanupFuture, ConstructionContext, Delivery,
        DependencyRequest, ErasedService, FactoryConstructionContext, FactoryFuture,
        FactoryInvoker, FactoryProvider, Injectable, InputPosition, Lifetime, Provider,
        ProviderCommon, ProviderSource, REFLECTED_PROVIDERS, ServiceIdentifier, ServiceSource,
        ServiceType, prepare_required,
    },
    ServiceProvider,
    scope::{ScopeLayer, ScopeProvider},
};

type Inject<T> = nestrs_core::__private::Inject<T>;

struct EdgeTransient {
    instance: usize,
}

struct EdgeRoot {
    first: Inject<EdgeTransient>,
    second: Inject<EdgeTransient>,
}

struct SyncFactoryTransient {
    instance: usize,
}

struct SyncFactoryRoot {
    transient: Inject<SyncFactoryTransient>,
}

struct ConsumerTransient {
    instance: usize,
}

struct FirstConsumer {
    transient: Inject<ConsumerTransient>,
}

struct SecondConsumer {
    transient: Inject<ConsumerTransient>,
}

struct MultiConsumerRoot {
    first: Inject<FirstConsumer>,
    second: Inject<SecondConsumer>,
}

struct ScopeApp;

struct ScopeTransient {
    instance: usize,
}

struct ScopeRoot {
    transient: Inject<ScopeTransient>,
}

struct InversionScoped;

struct InversionTransient {
    _scoped: Inject<InversionScoped>,
}

struct InversionRoot {
    _transient: Inject<InversionTransient>,
}

struct LegalScopeApp;

struct LegalScoped;

struct LegalTransient {
    scoped: Inject<LegalScoped>,
}

struct LegalScopeRoot {
    transient: Inject<LegalTransient>,
}

struct ShutdownTransient;

struct ShutdownRoot {
    _transient: Inject<ShutdownTransient>,
}

struct FactoryCleanupTransient;

struct FactoryCleanupRoot;

struct TransientRoot;

static EDGE_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SYNC_FACTORY_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static CONSUMER_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SCOPE_TRANSIENT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SHUTDOWN_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static SHUTDOWN_TEST_LOCK: Mutex<()> = Mutex::new(());

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the current-thread Tokio runtime should build for an async transient test")
        .block_on(future)
}

fn identifier<T>() -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::from(ServiceType::create::<T>())
}

fn source(line: u32) -> ServiceSource {
    ServiceSource::new("tests/transient_runtime.rs", line, 1)
}

fn common(lifetime: Lifetime, line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime,
        primary: false,
        source: source(line),
        cleanup: None,
    }
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

fn construct_edge_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(EdgeTransient {
        instance: EDGE_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_edge_root(mut context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(EdgeRoot {
        first: context.take::<EdgeTransient>(InputPosition(0))?,
        second: context.take::<EdgeTransient>(InputPosition(1))?,
    }))
}

fn construct_sync_factory_transient<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(SyncFactoryTransient {
        instance: SYNC_FACTORY_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_sync_factory_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(SyncFactoryRoot {
        transient: context.take::<SyncFactoryTransient>(InputPosition(0))?,
    }))
}

fn construct_consumer_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ConsumerTransient {
        instance: CONSUMER_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_first_consumer(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FirstConsumer {
        transient: context.take::<ConsumerTransient>(InputPosition(0))?,
    }))
}

fn construct_second_consumer(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(SecondConsumer {
        transient: context.take::<ConsumerTransient>(InputPosition(0))?,
    }))
}

fn construct_multi_consumer_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(MultiConsumerRoot {
        first: context.take::<FirstConsumer>(InputPosition(0))?,
        second: context.take::<SecondConsumer>(InputPosition(1))?,
    }))
}

fn construct_scope_app(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeApp))
}

fn construct_scope_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeTransient {
        instance: SCOPE_TRANSIENT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_scope_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeRoot {
        transient: context.take::<ScopeTransient>(InputPosition(0))?,
    }))
}

fn construct_inversion_scoped(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(InversionScoped))
}

fn construct_inversion_transient(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(InversionTransient {
        _scoped: context.take::<InversionScoped>(InputPosition(0))?,
    }))
}

fn construct_inversion_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(InversionRoot {
        _transient: context.take::<InversionTransient>(InputPosition(0))?,
    }))
}

fn construct_legal_scope_app(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(LegalScopeApp))
}

fn construct_legal_scoped(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(LegalScoped))
}

fn construct_legal_transient(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(LegalTransient {
        scoped: context.take::<LegalScoped>(InputPosition(0))?,
    }))
}

fn construct_legal_scope_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(LegalScopeRoot {
        transient: context.take::<LegalTransient>(InputPosition(0))?,
    }))
}

fn record_shutdown(event: &'static str) {
    SHUTDOWN_EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("shutdown event mutex should not be poisoned")
        .push(event);
}

impl Drop for ShutdownTransient {
    fn drop(&mut self) {
        record_shutdown("transient:drop");
    }
}

impl Drop for ShutdownRoot {
    fn drop(&mut self) {
        record_shutdown("root:drop");
    }
}

fn construct_shutdown_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownTransient))
}

fn construct_shutdown_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownRoot {
        _transient: context.take::<ShutdownTransient>(InputPosition(0))?,
    }))
}

fn cleanup_shutdown_transient() -> CleanupFuture {
    Box::pin(async { record_shutdown("transient:cleanup") })
}

fn construct_factory_cleanup_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FactoryCleanupTransient))
}

fn construct_factory_cleanup_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async move {
        let _transient = context.take::<FactoryCleanupTransient>(InputPosition(0))?;
        Ok(ErasedService::new(FactoryCleanupRoot))
    })
}

fn construct_transient_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(TransientRoot))
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn edge_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<EdgeTransient>(),
        common: transient_common(201),
        dependencies: Vec::new(),
        constructor: construct_edge_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn edge_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<EdgeRoot>(),
        common: common(Lifetime::Singleton, 210),
        dependencies: vec![
            dependency::<EdgeTransient>(0),
            dependency::<EdgeTransient>(1),
        ],
        constructor: construct_edge_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn sync_factory_transient_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<SyncFactoryTransient>(),
        common: transient_common(215),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Sync(construct_sync_factory_transient),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn sync_factory_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<SyncFactoryRoot>(),
        common: common(Lifetime::Singleton, 218),
        dependencies: vec![dependency::<SyncFactoryTransient>(0)],
        constructor: construct_sync_factory_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn consumer_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ConsumerTransient>(),
        common: transient_common(220),
        dependencies: Vec::new(),
        constructor: construct_consumer_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn first_consumer_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FirstConsumer>(),
        common: common(Lifetime::Singleton, 222),
        dependencies: vec![dependency::<ConsumerTransient>(0)],
        constructor: construct_first_consumer,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn second_consumer_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<SecondConsumer>(),
        common: common(Lifetime::Singleton, 224),
        dependencies: vec![dependency::<ConsumerTransient>(0)],
        constructor: construct_second_consumer,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn multi_consumer_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<MultiConsumerRoot>(),
        common: common(Lifetime::Singleton, 226),
        dependencies: vec![
            dependency::<FirstConsumer>(0),
            dependency::<SecondConsumer>(1),
        ],
        constructor: construct_multi_consumer_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeApp>(),
        common: common(Lifetime::Singleton, 220),
        dependencies: Vec::new(),
        constructor: construct_scope_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeTransient>(),
        common: transient_common(230),
        dependencies: Vec::new(),
        constructor: construct_scope_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeRoot>(),
        common: common(Lifetime::Scoped, 240),
        dependencies: vec![dependency::<ScopeTransient>(0)],
        constructor: construct_scope_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn inversion_scoped_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<InversionScoped>(),
        common: common(Lifetime::Scoped, 250),
        dependencies: Vec::new(),
        constructor: construct_inversion_scoped,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn inversion_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<InversionTransient>(),
        common: transient_common(260),
        dependencies: vec![dependency::<InversionScoped>(0)],
        constructor: construct_inversion_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn inversion_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<InversionRoot>(),
        common: common(Lifetime::Singleton, 270),
        dependencies: vec![dependency::<InversionTransient>(0)],
        constructor: construct_inversion_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn legal_scope_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<LegalScopeApp>(),
        common: common(Lifetime::Singleton, 280),
        dependencies: Vec::new(),
        constructor: construct_legal_scope_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn legal_scoped_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<LegalScoped>(),
        common: common(Lifetime::Scoped, 290),
        dependencies: Vec::new(),
        constructor: construct_legal_scoped,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn legal_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<LegalTransient>(),
        common: transient_common(300),
        dependencies: vec![dependency::<LegalScoped>(0)],
        constructor: construct_legal_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn legal_scope_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<LegalScopeRoot>(),
        common: common(Lifetime::Scoped, 310),
        dependencies: vec![dependency::<LegalTransient>(0)],
        constructor: construct_legal_scope_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownTransient>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_transient),
            ..transient_common(320)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownRoot>(),
        common: common(Lifetime::Singleton, 330),
        dependencies: vec![dependency::<ShutdownTransient>(0)],
        constructor: construct_shutdown_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn factory_cleanup_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FactoryCleanupTransient>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_transient),
            ..transient_common(340)
        },
        dependencies: Vec::new(),
        constructor: construct_factory_cleanup_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn factory_cleanup_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<FactoryCleanupRoot>(),
        common: common(Lifetime::Singleton, 350),
        dependencies: vec![dependency::<FactoryCleanupTransient>(0)],
        invoker: FactoryInvoker::Async(construct_factory_cleanup_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn transient_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<TransientRoot>(),
        common: transient_common(360),
        dependencies: Vec::new(),
        constructor: construct_transient_root,
    })
}

#[test]
fn every_transient_injection_edge_builds_a_distinct_instance() {
    EDGE_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::<EdgeRoot>::build().expect("root should build");
    let root = provider.root();

    assert_eq!(root.first.instance, 1);
    assert_eq!(root.second.instance, 2);
    assert_ne!(
        &*root.first as *const EdgeTransient, &*root.second as *const EdgeTransient,
        "two fields requesting the same transient token must not share an occurrence"
    );
}

#[test]
fn synchronous_factory_can_produce_a_field_owned_transient() {
    SYNC_FACTORY_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::<SyncFactoryRoot>::build()
        .expect("a synchronous transient factory should run through the sync activation path");

    assert_eq!(provider.root().transient.instance, 1);
    assert_eq!(
        SYNC_FACTORY_TRANSIENT_CONSTRUCTIONS.load(Ordering::SeqCst),
        1
    );
}

#[test]
fn different_consumers_do_not_share_a_transient_occurrence() {
    CONSUMER_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::<MultiConsumerRoot>::build()
        .expect("each consumer should receive its own transient occurrence");
    let root = provider.root();

    assert_eq!(root.first.transient.instance, 1);
    assert_eq!(root.second.transient.instance, 2);
    assert_ne!(
        &*root.first.transient as *const ConsumerTransient,
        &*root.second.transient as *const ConsumerTransient,
        "the same transient token must not be shared across two consumers"
    );
}

#[test]
fn scoped_transients_are_rebuilt_for_each_scope() {
    SCOPE_TRANSIENT_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    let provider = ScopeProvider::<ScopeApp, ScopeLayer<ScopeRoot>>::build()
        .expect("scope plan should compile");
    let first = {
        let scope = provider.create_scope().expect("first scope should build");
        scope.root().transient.instance
    };
    let second = {
        let scope = provider.create_scope().expect("second scope should build");
        scope.root().transient.instance
    };

    assert_eq!((first, second), (1, 2));
}

#[test]
fn singleton_owned_transient_cannot_capture_scoped_service() {
    let error = match ServiceProvider::<InversionRoot>::build() {
        Ok(_) => panic!("a singleton-owned transient must not retain a scoped Inject"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("transient 子树不能依赖 Scoped provider") && message.contains("Singleton"),
        "unexpected compile error: {message}"
    );
}

#[test]
fn scoped_owned_transient_can_capture_scoped_service() {
    let provider = ScopeProvider::<LegalScopeApp, ScopeLayer<LegalScopeRoot>>::build()
        .expect("scoped transient plan should compile");
    let scope = provider.create_scope().expect("scope should build");
    let _scoped = &*scope.root().transient.scoped;
}

#[test]
fn async_shutdown_runs_a_field_transient_hook_after_its_owner_drop() {
    let _lock = SHUTDOWN_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    SHUTDOWN_EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .expect("shutdown event mutex should not be poisoned")
        .clear();

    let provider = block_on(ServiceProvider::<ShutdownRoot>::build_async())
        .expect("async build should accept field transient cleanup");
    block_on(provider.shutdown());

    assert_eq!(
        *SHUTDOWN_EVENTS
            .get()
            .expect("shutdown events should initialize")
            .lock()
            .expect("shutdown event mutex should not be poisoned"),
        vec!["root:drop", "transient:cleanup", "transient:drop"]
    );
}

#[test]
fn async_build_rejects_cleanup_in_a_factory_parameter_transient_subtree() {
    let error = match block_on(ServiceProvider::<FactoryCleanupRoot>::build_async()) {
        Ok(_) => panic!("factory parameter transient cleanup has no public shutdown owner"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("transient 参数子树包含 cleanup provider"),
        "unexpected build error: {error}"
    );
}

#[test]
fn transient_cannot_be_a_public_root() {
    let error = match ServiceProvider::<TransientRoot>::build() {
        Ok(_) => panic!("a public root must provide a stable singleton borrow"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("root 服务")
            && message.contains("Singleton")
            && message.contains("Transient"),
        "unexpected build error: {message}"
    );
}
