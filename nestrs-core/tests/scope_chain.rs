use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};

use nestrs_core::{
    __private::{
        ActivationError, ClassProvider, CleanupFuture, ConstructionContext, Delivery,
        DependencyRequest, ErasedService, FactoryConstructionContext, FactoryFuture,
        FactoryInvoker, FactoryProvider, Injectable, InputPosition, Lifetime, Provider,
        ProviderCommon, ProviderSource, REFLECTED_PROVIDERS, ServiceIdentifier, ServiceSource,
        ServiceType, prepare_required,
    },
    scope::{ScopeEnd, ScopeLayer, ScopeProvider},
};

type Inject<T> = nestrs_core::__private::Inject<T>;

struct ChainSingleton {
    instance: usize,
}

struct ChainApp {
    singleton: Inject<ChainSingleton>,
}

struct ChainRequestRoot {
    singleton: Inject<ChainSingleton>,
    instance: usize,
}

struct ChainTransactionRoot {
    singleton: Inject<ChainSingleton>,
    request: Inject<ChainRequestRoot>,
    instance: usize,
}

struct ChainCommandRoot {
    singleton: Inject<ChainSingleton>,
    request: Inject<ChainRequestRoot>,
    transaction: Inject<ChainTransactionRoot>,
    instance: usize,
}

struct AncestorOwnerRoot {
    _owned_child: Inject<OwnedChildRoot>,
}

struct OwnedChildRoot;

struct AsyncChainApp;

struct AsyncChainRequestRoot;

struct AsyncChainTransactionRoot {
    label: &'static str,
}

struct AsyncChainCommandRoot {
    transaction: Inject<AsyncChainTransactionRoot>,
}

struct ShutdownChainApp;

struct ShutdownRequestRoot;

struct ShutdownTransactionRoot {
    _request: Inject<ShutdownRequestRoot>,
}

struct ShutdownCommandRoot {
    _transaction: Inject<ShutdownTransactionRoot>,
}

struct FailureChainSingleton {
    label: &'static str,
}

struct FailureChainApp {
    singleton: Inject<FailureChainSingleton>,
}

struct FailureRequestRoot {
    label: &'static str,
}

struct FailureTransactionLocal;

struct FailureTransactionRoot;

struct IndirectOuterRoot {
    _intermediate: Inject<IndirectIntermediate>,
}

struct IndirectIntermediate {
    _child: Inject<IndirectChildRoot>,
}

struct IndirectChildRoot;

struct CancellationChainApp {
    label: &'static str,
}

struct CancellationRequestRoot {
    label: &'static str,
}

struct CancellationTransactionLocal;

struct CancellationTransactionRoot;

type ThreeLayerChain = ScopeLayer<
    ChainRequestRoot,
    ScopeLayer<ChainTransactionRoot, ScopeLayer<ChainCommandRoot, ScopeEnd>>,
>;
type DuplicateRootChain = ScopeLayer<ChainRequestRoot, ScopeLayer<ChainRequestRoot, ScopeEnd>>;
type AncestorOwnedRootChain = ScopeLayer<AncestorOwnerRoot, ScopeLayer<OwnedChildRoot, ScopeEnd>>;
type AsyncThreeLayerChain = ScopeLayer<
    AsyncChainRequestRoot,
    ScopeLayer<AsyncChainTransactionRoot, ScopeLayer<AsyncChainCommandRoot, ScopeEnd>>,
>;
type ShutdownThreeLayerChain = ScopeLayer<
    ShutdownRequestRoot,
    ScopeLayer<ShutdownTransactionRoot, ScopeLayer<ShutdownCommandRoot, ScopeEnd>>,
>;
type FailureTwoLayerChain =
    ScopeLayer<FailureRequestRoot, ScopeLayer<FailureTransactionRoot, ScopeEnd>>;
type IndirectInversionChain =
    ScopeLayer<IndirectOuterRoot, ScopeLayer<IndirectChildRoot, ScopeEnd>>;
type CancellationTwoLayerChain =
    ScopeLayer<CancellationRequestRoot, ScopeLayer<CancellationTransactionRoot, ScopeEnd>>;

static TEST_LOCK: Mutex<()> = Mutex::new(());
static SINGLETON_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static REQUEST_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static TRANSACTION_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static COMMAND_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SHUTDOWN_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static FAILURE_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static CANCELLATION_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static CANCELLATION_CHILD_STARTED: AtomicBool = AtomicBool::new(false);

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the current-thread Tokio runtime should build for an async scope-chain test")
        .block_on(future)
}

fn identifier<T>() -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::from(ServiceType::create::<T>())
}

fn source(line: u32) -> ServiceSource {
    ServiceSource::new("tests/scope_chain.rs", line, 1)
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

fn shutdown_events() -> &'static Mutex<Vec<&'static str>> {
    SHUTDOWN_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_shutdown(event: &'static str) {
    shutdown_events()
        .lock()
        .expect("scope-chain shutdown event mutex should not be poisoned")
        .push(event);
}

fn reset_shutdown_events() {
    shutdown_events()
        .lock()
        .expect("scope-chain shutdown event mutex should not be poisoned")
        .clear();
}

fn failure_events() -> &'static Mutex<Vec<&'static str>> {
    FAILURE_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_failure(event: &'static str) {
    failure_events()
        .lock()
        .expect("scope-chain failure event mutex should not be poisoned")
        .push(event);
}

fn cancellation_events() -> &'static Mutex<Vec<&'static str>> {
    CANCELLATION_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_cancellation(event: &'static str) {
    cancellation_events()
        .lock()
        .expect("scope-chain cancellation event mutex should not be poisoned")
        .push(event);
}

fn construct_chain_singleton(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ChainSingleton {
        instance: SINGLETON_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_chain_app(mut context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ChainApp {
        singleton: context.take::<ChainSingleton>(InputPosition(0))?,
    }))
}

fn construct_chain_request(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ChainRequestRoot {
        singleton: context.take::<ChainSingleton>(InputPosition(0))?,
        instance: REQUEST_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_chain_transaction(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ChainTransactionRoot {
        singleton: context.take::<ChainSingleton>(InputPosition(0))?,
        request: context.take::<ChainRequestRoot>(InputPosition(1))?,
        instance: TRANSACTION_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_chain_command(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ChainCommandRoot {
        singleton: context.take::<ChainSingleton>(InputPosition(0))?,
        request: context.take::<ChainRequestRoot>(InputPosition(1))?,
        transaction: context.take::<ChainTransactionRoot>(InputPosition(2))?,
        instance: COMMAND_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_ancestor_owner(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AncestorOwnerRoot {
        _owned_child: context.take::<OwnedChildRoot>(InputPosition(0))?,
    }))
}

fn construct_owned_child(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(OwnedChildRoot))
}

fn construct_async_chain_app(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncChainApp))
}

fn construct_async_chain_request(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncChainRequestRoot))
}

fn construct_async_chain_transaction<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async {
        Ok(ErasedService::new(AsyncChainTransactionRoot {
            label: "async transaction",
        }))
    })
}

fn construct_async_chain_command(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncChainCommandRoot {
        transaction: context.take::<AsyncChainTransactionRoot>(InputPosition(0))?,
    }))
}

fn construct_shutdown_chain_app(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownChainApp))
}

fn construct_shutdown_request(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownRequestRoot))
}

fn construct_shutdown_transaction(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownTransactionRoot {
        _request: context.take::<ShutdownRequestRoot>(InputPosition(0))?,
    }))
}

fn construct_shutdown_command(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownCommandRoot {
        _transaction: context.take::<ShutdownTransactionRoot>(InputPosition(0))?,
    }))
}

fn construct_failure_chain_singleton(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FailureChainSingleton {
        label: "failure parent singleton",
    }))
}

fn construct_failure_chain_app(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FailureChainApp {
        singleton: context.take::<FailureChainSingleton>(InputPosition(0))?,
    }))
}

fn construct_failure_request(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FailureRequestRoot {
        label: "failure request parent",
    }))
}

fn construct_failure_transaction_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FailureTransactionLocal))
}

fn construct_failure_transaction_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    let _request = context.take::<FailureRequestRoot>(InputPosition(0))?;
    let _local = context.take::<FailureTransactionLocal>(InputPosition(1))?;
    Err(ActivationError::FactoryFailed {
        provider: "construct_failure_transaction_root",
        provider_source: source(620),
    })
}

fn construct_indirect_outer(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(IndirectOuterRoot {
        _intermediate: context.take::<IndirectIntermediate>(InputPosition(0))?,
    }))
}

fn construct_indirect_intermediate(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(IndirectIntermediate {
        _child: context.take::<IndirectChildRoot>(InputPosition(0))?,
    }))
}

fn construct_indirect_child(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(IndirectChildRoot))
}

fn construct_cancellation_chain_app(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(CancellationChainApp {
        label: "cancellation app",
    }))
}

fn construct_cancellation_request(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(CancellationRequestRoot {
        label: "cancellation request",
    }))
}

fn construct_cancellation_transaction_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(CancellationTransactionLocal))
}

/// This future keeps the parent Scoped token and the child-local token live until its caller
/// drops `create_child_scope_async()`. That makes cancellation order observable without timing.
struct CancellationChildPendingFuture<'frame> {
    _request: nestrs_core::__private::Inject<
        CancellationRequestRoot,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
    _local: nestrs_core::__private::Inject<
        CancellationTransactionLocal,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
}

impl Future for CancellationChildPendingFuture<'_> {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

impl Drop for CancellationChildPendingFuture<'_> {
    fn drop(&mut self) {
        record_cancellation("transaction factory future:drop");
    }
}

fn construct_cancellation_transaction_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    let request = match context.take::<CancellationRequestRoot>(InputPosition(0)) {
        Ok(request) => request,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    let local = match context.take::<CancellationTransactionLocal>(InputPosition(1)) {
        Ok(local) => local,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    CANCELLATION_CHILD_STARTED.store(true, Ordering::SeqCst);
    Box::pin(CancellationChildPendingFuture {
        _request: request,
        _local: local,
    })
}

impl Drop for ShutdownChainApp {
    fn drop(&mut self) {
        record_shutdown("app:drop");
    }
}

impl Drop for ShutdownRequestRoot {
    fn drop(&mut self) {
        record_shutdown("request:drop");
    }
}

impl Drop for ShutdownTransactionRoot {
    fn drop(&mut self) {
        record_shutdown("transaction:drop");
    }
}

impl Drop for ShutdownCommandRoot {
    fn drop(&mut self) {
        record_shutdown("command:drop");
    }
}

impl Drop for FailureTransactionLocal {
    fn drop(&mut self) {
        record_failure("transaction local:drop");
    }
}

impl Drop for CancellationTransactionLocal {
    fn drop(&mut self) {
        record_cancellation("transaction local:drop");
    }
}

fn cleanup_shutdown_chain_app() -> CleanupFuture {
    Box::pin(async { record_shutdown("app:cleanup") })
}

fn cleanup_shutdown_request() -> CleanupFuture {
    Box::pin(async { record_shutdown("request:cleanup") })
}

fn cleanup_shutdown_transaction() -> CleanupFuture {
    Box::pin(async { record_shutdown("transaction:cleanup") })
}

fn cleanup_shutdown_command() -> CleanupFuture {
    Box::pin(async { record_shutdown("command:cleanup") })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn chain_singleton_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ChainSingleton>(),
        common: singleton_common(200),
        dependencies: Vec::new(),
        constructor: construct_chain_singleton,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn chain_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ChainApp>(),
        common: singleton_common(210),
        dependencies: vec![dependency::<ChainSingleton>(0)],
        constructor: construct_chain_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn chain_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ChainRequestRoot>(),
        common: scoped_common(220),
        dependencies: vec![dependency::<ChainSingleton>(0)],
        constructor: construct_chain_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn chain_transaction_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ChainTransactionRoot>(),
        common: scoped_common(230),
        dependencies: vec![
            dependency::<ChainSingleton>(0),
            dependency::<ChainRequestRoot>(1),
        ],
        constructor: construct_chain_transaction,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn chain_command_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ChainCommandRoot>(),
        common: scoped_common(240),
        dependencies: vec![
            dependency::<ChainSingleton>(0),
            dependency::<ChainRequestRoot>(1),
            dependency::<ChainTransactionRoot>(2),
        ],
        constructor: construct_chain_command,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn ancestor_owner_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AncestorOwnerRoot>(),
        common: scoped_common(250),
        dependencies: vec![dependency::<OwnedChildRoot>(0)],
        constructor: construct_ancestor_owner,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn owned_child_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<OwnedChildRoot>(),
        common: scoped_common(260),
        dependencies: Vec::new(),
        constructor: construct_owned_child,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_chain_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncChainApp>(),
        common: singleton_common(270),
        dependencies: Vec::new(),
        constructor: construct_async_chain_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_chain_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncChainRequestRoot>(),
        common: scoped_common(280),
        dependencies: Vec::new(),
        constructor: construct_async_chain_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_chain_transaction_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<AsyncChainTransactionRoot>(),
        common: scoped_common(290),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_async_chain_transaction),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_chain_command_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncChainCommandRoot>(),
        common: scoped_common(300),
        dependencies: vec![dependency::<AsyncChainTransactionRoot>(0)],
        constructor: construct_async_chain_command,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_chain_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownChainApp>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_chain_app),
            ..singleton_common(500)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_chain_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownRequestRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_request),
            ..scoped_common(510)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_transaction_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownTransactionRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_transaction),
            ..scoped_common(520)
        },
        dependencies: vec![dependency::<ShutdownRequestRoot>(0)],
        constructor: construct_shutdown_transaction,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_command_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownCommandRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_command),
            ..scoped_common(530)
        },
        dependencies: vec![dependency::<ShutdownTransactionRoot>(0)],
        constructor: construct_shutdown_command,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_chain_singleton_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureChainSingleton>(),
        common: singleton_common(600),
        dependencies: Vec::new(),
        constructor: construct_failure_chain_singleton,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_chain_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureChainApp>(),
        common: singleton_common(610),
        dependencies: vec![dependency::<FailureChainSingleton>(0)],
        constructor: construct_failure_chain_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureRequestRoot>(),
        common: scoped_common(620),
        dependencies: Vec::new(),
        constructor: construct_failure_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_transaction_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureTransactionLocal>(),
        common: scoped_common(630),
        dependencies: Vec::new(),
        constructor: construct_failure_transaction_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_transaction_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureTransactionRoot>(),
        common: scoped_common(640),
        dependencies: vec![
            dependency::<FailureRequestRoot>(0),
            dependency::<FailureTransactionLocal>(1),
        ],
        constructor: construct_failure_transaction_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn indirect_outer_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<IndirectOuterRoot>(),
        common: scoped_common(700),
        dependencies: vec![dependency::<IndirectIntermediate>(0)],
        constructor: construct_indirect_outer,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn indirect_intermediate_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<IndirectIntermediate>(),
        common: transient_common(710),
        dependencies: vec![dependency::<IndirectChildRoot>(0)],
        constructor: construct_indirect_intermediate,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn indirect_child_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<IndirectChildRoot>(),
        common: scoped_common(720),
        dependencies: Vec::new(),
        constructor: construct_indirect_child,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_chain_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<CancellationChainApp>(),
        common: singleton_common(800),
        dependencies: Vec::new(),
        constructor: construct_cancellation_chain_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_request_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<CancellationRequestRoot>(),
        common: scoped_common(810),
        dependencies: Vec::new(),
        constructor: construct_cancellation_request,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_transaction_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<CancellationTransactionLocal>(),
        common: scoped_common(820),
        dependencies: Vec::new(),
        constructor: construct_cancellation_transaction_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_transaction_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<CancellationTransactionRoot>(),
        common: scoped_common(830),
        dependencies: vec![
            dependency::<CancellationRequestRoot>(0),
            dependency::<CancellationTransactionLocal>(1),
        ],
        invoker: FactoryInvoker::Async(construct_cancellation_transaction_root),
    })
}

#[test]
fn static_scope_chain_reuses_ancestors_and_isolates_siblings() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    SINGLETON_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    REQUEST_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    TRANSACTION_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    COMMAND_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider = ScopeProvider::<ChainApp, ThreeLayerChain>::build()
        .expect("the static three-layer scope chain should compile");
    let first_request = provider
        .create_scope()
        .expect("request scope should activate");
    let first_transaction = first_request
        .create_child_scope()
        .expect("transaction scope should activate");
    let first_command = first_transaction
        .create_child_scope()
        .expect("command scope should activate");
    let second_command = first_transaction
        .create_child_scope()
        .expect("a sibling command scope should activate independently");
    let second_transaction = first_request
        .create_child_scope()
        .expect("a sibling transaction scope should activate independently");
    let third_command = second_transaction
        .create_child_scope()
        .expect("a command scope below the second transaction should activate");
    let second_request = provider
        .create_scope()
        .expect("a second request scope should activate independently");
    let third_transaction = second_request
        .create_child_scope()
        .expect("a transaction below the second request should activate");
    let fourth_command = third_transaction
        .create_child_scope()
        .expect("a command below the second request should activate");

    assert_eq!(provider.root().singleton.instance, 1);
    assert!(std::ptr::eq(
        &*provider.root().singleton,
        &*first_request.root().singleton
    ));
    assert!(std::ptr::eq(
        &*provider.root().singleton,
        &*first_transaction.root().singleton
    ));
    assert!(std::ptr::eq(
        &*provider.root().singleton,
        &*first_command.root().singleton
    ));
    assert!(std::ptr::eq(
        first_request.root(),
        &*first_transaction.root().request
    ));
    assert!(std::ptr::eq(
        first_request.root(),
        &*first_command.root().request
    ));
    assert!(std::ptr::eq(
        first_transaction.root(),
        &*first_command.root().transaction
    ));
    assert!(std::ptr::eq(
        first_transaction.root(),
        &*second_command.root().transaction
    ));
    assert!(std::ptr::eq(
        first_request.root(),
        &*second_transaction.root().request
    ));
    assert!(std::ptr::eq(
        second_transaction.root(),
        &*third_command.root().transaction
    ));
    assert!(std::ptr::eq(
        second_request.root(),
        &*third_transaction.root().request
    ));
    assert!(std::ptr::eq(
        third_transaction.root(),
        &*fourth_command.root().transaction
    ));

    assert_ne!(
        first_request.root().instance,
        second_request.root().instance
    );
    assert_ne!(
        first_transaction.root().instance,
        second_transaction.root().instance
    );
    assert_ne!(
        first_transaction.root().instance,
        third_transaction.root().instance
    );
    assert_ne!(
        first_command.root().instance,
        second_command.root().instance
    );
    assert_ne!(first_command.root().instance, third_command.root().instance);
    assert_ne!(
        first_command.root().instance,
        fourth_command.root().instance
    );
    assert_eq!(SINGLETON_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(REQUEST_CONSTRUCTIONS.load(Ordering::SeqCst), 2);
    assert_eq!(TRANSACTION_CONSTRUCTIONS.load(Ordering::SeqCst), 3);
    assert_eq!(COMMAND_CONSTRUCTIONS.load(Ordering::SeqCst), 4);
}

#[test]
fn scope_chain_rejects_duplicate_and_ancestor_owned_roots() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let duplicate_error = match ScopeProvider::<ChainApp, DuplicateRootChain>::build() {
        Ok(_) => panic!("a scope root may only be owned by its first chain frame"),
        Err(error) => error,
    };
    assert!(
        duplicate_error
            .to_string()
            .contains("已经由祖先 scope root"),
        "unexpected duplicate root error: {duplicate_error}"
    );

    let ancestor_error = match ScopeProvider::<ChainApp, AncestorOwnedRootChain>::build() {
        Ok(_) => panic!("a child root already reachable from its ancestor must be rejected"),
        Err(error) => error,
    };
    assert!(
        ancestor_error.to_string().contains("已经由祖先 scope root"),
        "unexpected ancestor root error: {ancestor_error}"
    );
}

#[test]
fn async_scope_chain_activates_async_child_factories_and_sync_rejects_them() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let sync_error = match ScopeProvider::<AsyncChainApp, AsyncThreeLayerChain>::build() {
        Ok(_) => panic!("a sync scope-chain build must reject a reachable async factory"),
        Err(error) => error,
    };
    assert!(
        sync_error.to_string().contains("使用了 async factory"),
        "unexpected sync scope-chain error: {sync_error}"
    );

    let provider = block_on(ScopeProvider::<AsyncChainApp, AsyncThreeLayerChain>::build_async())
        .expect("the async scope-chain build should accept the reachable async factory");
    let request =
        block_on(provider.create_scope_async()).expect("the async request scope should activate");
    let transaction = block_on(request.create_child_scope_async())
        .expect("the async transaction scope should activate its factory");
    let command = block_on(transaction.create_child_scope_async())
        .expect("the async command scope should receive its parent transaction");

    assert_eq!(transaction.root().label, "async transaction");
    assert!(std::ptr::eq(
        transaction.root(),
        &*command.root().transaction
    ));
}

#[test]
fn scope_chain_shutdown_is_consumed_child_to_parent_without_crossing_arenas() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_shutdown_events();

    let provider =
        block_on(ScopeProvider::<ShutdownChainApp, ShutdownThreeLayerChain>::build_async())
            .expect("async scope-chain build should accept cleanup hooks");
    let request = block_on(provider.create_scope_async())
        .expect("request scope should accept its cleanup hook");
    let transaction = block_on(request.create_child_scope_async())
        .expect("transaction scope should accept its cleanup hook");
    let command = block_on(transaction.create_child_scope_async())
        .expect("command scope should accept its cleanup hook");

    block_on(command.shutdown());
    assert_eq!(
        *shutdown_events()
            .lock()
            .expect("scope-chain shutdown event mutex should not be poisoned"),
        ["command:cleanup", "command:drop"],
        "shutting down a child scope must not consume its parent arenas"
    );

    block_on(transaction.shutdown());
    assert_eq!(
        *shutdown_events()
            .lock()
            .expect("scope-chain shutdown event mutex should not be poisoned"),
        [
            "command:cleanup",
            "command:drop",
            "transaction:cleanup",
            "transaction:drop",
        ]
    );

    block_on(request.shutdown());
    assert_eq!(
        *shutdown_events()
            .lock()
            .expect("scope-chain shutdown event mutex should not be poisoned"),
        [
            "command:cleanup",
            "command:drop",
            "transaction:cleanup",
            "transaction:drop",
            "request:cleanup",
            "request:drop",
        ]
    );

    block_on(provider.shutdown());
    assert_eq!(
        *shutdown_events()
            .lock()
            .expect("scope-chain shutdown event mutex should not be poisoned"),
        [
            "command:cleanup",
            "command:drop",
            "transaction:cleanup",
            "transaction:drop",
            "request:cleanup",
            "request:drop",
            "app:cleanup",
            "app:drop",
        ],
        "each arena must run only its own cleanup immediately before dropping its service"
    );
}

#[test]
fn ordinary_scope_chain_drop_never_drives_cleanup_hooks() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_shutdown_events();

    {
        let provider =
            block_on(ScopeProvider::<ShutdownChainApp, ShutdownThreeLayerChain>::build_async())
                .expect("async scope-chain build should accept cleanup hooks");
        let request =
            block_on(provider.create_scope_async()).expect("request scope should activate");
        let transaction = block_on(request.create_child_scope_async())
            .expect("transaction scope should activate");
        let _command = block_on(transaction.create_child_scope_async())
            .expect("command scope should activate");

        assert!(
            shutdown_events()
                .lock()
                .expect("scope-chain shutdown event mutex should not be poisoned")
                .is_empty(),
            "cleanup hooks must not run during normal construction"
        );
    }

    assert_eq!(
        *shutdown_events()
            .lock()
            .expect("scope-chain shutdown event mutex should not be poisoned"),
        [
            "command:drop",
            "transaction:drop",
            "request:drop",
            "app:drop",
        ],
        "ordinary Drop must release every arena without driving its async cleanup hook"
    );
}

#[test]
fn failed_child_scope_rolls_back_only_its_own_local_arena() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    failure_events()
        .lock()
        .expect("scope-chain failure event mutex should not be poisoned")
        .clear();

    let provider = ScopeProvider::<FailureChainApp, FailureTwoLayerChain>::build()
        .expect("the singleton and parent request scope should compile");
    let request = provider
        .create_scope()
        .expect("the parent request scope should activate before its child fails");
    let error = match request.create_child_scope() {
        Ok(_) => panic!("the intentionally failing transaction root must abort its own scope"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("激活 provider"),
        "unexpected child scope activation error: {error}"
    );
    assert_eq!(provider.root().singleton.label, "failure parent singleton");
    assert_eq!(request.root().label, "failure request parent");
    assert_eq!(
        *failure_events()
            .lock()
            .expect("scope-chain failure event mutex should not be poisoned"),
        ["transaction local:drop"],
        "only the committed local node in the failed child Arena may roll back"
    );
}

#[test]
fn indirect_ancestor_capture_of_a_declared_child_root_is_a_lifetime_inversion() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    let error = match ScopeProvider::<ChainApp, IndirectInversionChain>::build() {
        Ok(_) => panic!("an ancestor transient subtree must not capture a later scoped root"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("不能依赖后代 scope root"),
        "unexpected scope lifetime inversion error: {error}"
    );
}

#[test]
fn cancelling_async_child_scope_rolls_back_only_child_work_and_keeps_ancestors_usable() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    cancellation_events()
        .lock()
        .expect("scope-chain cancellation event mutex should not be poisoned")
        .clear();
    CANCELLATION_CHILD_STARTED.store(false, Ordering::SeqCst);

    let provider = block_on(ScopeProvider::<
        CancellationChainApp,
        CancellationTwoLayerChain,
    >::build_async())
    .expect("the async provider and parent request scope should compile");
    let request = block_on(provider.create_scope_async())
        .expect("the parent request scope should activate before child cancellation");
    block_on(async {
        let mut creation = Box::pin(request.create_child_scope_async());

        for _ in 0..128 {
            std::future::poll_fn(|context| match creation.as_mut().poll(context) {
                Poll::Ready(_) => {
                    panic!("the cancellation fixture's transaction factory must remain pending")
                }
                Poll::Pending => Poll::Ready(()),
            })
            .await;
            if CANCELLATION_CHILD_STARTED.load(Ordering::SeqCst) {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert!(
            CANCELLATION_CHILD_STARTED.load(Ordering::SeqCst),
            "the child factory must be in flight before create_child_scope_async is cancelled"
        );
        drop(creation);

        for _ in 0..128 {
            if *cancellation_events()
                .lock()
                .expect("scope-chain cancellation event mutex should not be poisoned")
                == ["transaction factory future:drop", "transaction local:drop"]
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("Tokio abort should eventually roll back the child scope's local Arena");
    });

    assert_eq!(provider.root().label, "cancellation app");
    assert_eq!(request.root().label, "cancellation request");
    assert_eq!(
        *cancellation_events()
            .lock()
            .expect("scope-chain cancellation event mutex should not be poisoned"),
        ["transaction factory future:drop", "transaction local:drop"],
        "cancellation must drop in-flight child work before rolling back only its local Arena"
    );
}
