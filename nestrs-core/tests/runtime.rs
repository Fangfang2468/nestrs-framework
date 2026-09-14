use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use nestrs_core::{
    __private::{
        ActivationError, ArenaServiceRef, BoundKeyPolicy, ClassProvider, CleanupFuture,
        ConstructionContext, Delivery, DependencyRequest, ErasedService,
        FactoryConstructionContext, FactoryFuture, FactoryInvoker, FactoryProvider, Injectable,
        InputPosition, Lifetime, Provider, ProviderCommon, ProviderSource, ServiceIdentifier,
        ServiceKey, ServiceSource, ServiceType, TraitBinding, prepare_bound_optional,
        prepare_bound_required, prepare_optional, prepare_required,
    },
    ServiceProvider,
    scope::{ScopeLayer, ScopeProvider},
};

struct Database {
    label: &'static str,
}

struct Repository {
    database: nestrs_core::__private::Inject<Database>,
}

struct Produced {
    database_label: &'static str,
}

struct FactoryRoot {
    label: &'static str,
}

trait Port: Send + Sync {
    fn label(&self) -> &'static str;
}

struct Adapter;

impl Port for Adapter {
    fn label(&self) -> &'static str {
        "adapter"
    }
}

struct OptionalService;

trait KeyedPort: Send + Sync {
    fn label(&self) -> &'static str;
}

struct KeyedAdapter;

impl KeyedPort for KeyedAdapter {
    fn label(&self) -> &'static str {
        "keyed adapter"
    }
}

struct KeyedTraitRoot {
    port: nestrs_core::__private::Inject<dyn KeyedPort>,
}

struct Generic<T> {
    label: &'static str,
    _marker: PhantomData<T>,
}

struct GenericArgument;

struct GenericRoot {
    first: nestrs_core::__private::Inject<Generic<GenericArgument>>,
    second: nestrs_core::__private::Inject<Generic<GenericArgument>>,
}

struct RuntimeRoot {
    repository: nestrs_core::__private::Inject<Repository>,
    produced: nestrs_core::__private::Inject<Produced>,
    port: nestrs_core::__private::Inject<dyn Port>,
    optional: Option<nestrs_core::__private::Inject<OptionalService>>,
}

struct ScopedRoot;

struct RollbackFirst;

struct RollbackSecond;

struct FailingRoot;

struct AsyncFactoryPublicRoot {
    label: &'static str,
}

struct ConcurrentAsyncLeft {
    label: &'static str,
}

struct ConcurrentAsyncRight {
    label: &'static str,
}

struct ConcurrentAsyncRoot {
    left: nestrs_core::__private::Inject<ConcurrentAsyncLeft>,
    right: nestrs_core::__private::Inject<ConcurrentAsyncRight>,
}

struct OrderedAsyncDependency;

struct OrderedAsyncRoot;

struct AsyncRollbackLeaf;

struct AsyncRollbackMiddle {
    _leaf: nestrs_core::__private::Inject<AsyncRollbackLeaf>,
}

struct AsyncRollbackFailureRoot;

struct StableFailureFirst;

struct StableFailureSecond;

struct StableFailureRoot;

struct CancellationDependency;

struct CancellationRoot;

struct ScopeIntegrationAppRoot {
    label: &'static str,
}

struct ScopeIntegrationSingleton {
    instance: usize,
}

struct ScopeIntegrationLocal {
    instance: usize,
}

struct ScopeIntegrationRequestRoot {
    singleton: nestrs_core::__private::Inject<ScopeIntegrationSingleton>,
    local: nestrs_core::__private::Inject<ScopeIntegrationLocal>,
}

struct AsyncScopeIntegrationAppRoot;

struct AsyncScopeIntegrationRoot {
    label: &'static str,
}

struct ScopeFailureAppRoot {
    singleton: nestrs_core::__private::Inject<ScopeFailureSingleton>,
}

struct ScopeFailureSingleton {
    label: &'static str,
}

struct ScopeFailureLocal;

struct ScopeFailureRoot;

struct ScopedConcurrentAppRoot;

struct ScopedConcurrentLeft {
    label: &'static str,
}

struct ScopedConcurrentRight {
    label: &'static str,
}

struct ScopedConcurrentRoot {
    left: nestrs_core::__private::Inject<ScopedConcurrentLeft>,
    right: nestrs_core::__private::Inject<ScopedConcurrentRight>,
}

struct ScopedCancellationAppRoot {
    parent: nestrs_core::__private::Inject<ScopedCancellationParentSingleton>,
}

struct ScopedCancellationParentSingleton {
    label: &'static str,
}

struct ScopedCancellationLocal;

struct ScopedCancellationRoot;

struct ShutdownDependency;

struct ShutdownRoot {
    _dependency: nestrs_core::__private::Inject<ShutdownDependency>,
}

struct ShutdownFailureRoot;

struct ShutdownScopeAppRoot;

struct ShutdownScopeLocal;

struct ShutdownScopeRoot {
    _app: nestrs_core::__private::Inject<ShutdownScopeAppRoot>,
    _local: nestrs_core::__private::Inject<ShutdownScopeLocal>,
}

struct ShutdownCancellationRoot;

struct ShutdownAsyncFactoryRoot;

static GENERIC_MATERIALIZE_CALLS: AtomicUsize = AtomicUsize::new(0);
static GENERIC_MATERIALIZE_TEST_LOCK: Mutex<()> = Mutex::new(());
static ROLLBACK_DROP_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static ASYNC_CONCURRENT_STATE: OnceLock<AsyncConcurrentState> = OnceLock::new();
static ORDERED_ASYNC_DEPENDENCY_READY: AtomicBool = AtomicBool::new(false);
static ORDERED_ASYNC_ROOT_STARTED: AtomicBool = AtomicBool::new(false);
static ORDERED_ASYNC_ROOT_STARTED_EARLY: AtomicBool = AtomicBool::new(false);
static ASYNC_ROLLBACK_DROP_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static STABLE_FAILURE_STARTED: AtomicUsize = AtomicUsize::new(0);
static STABLE_FAILURE_SECOND_RETURNED: AtomicBool = AtomicBool::new(false);
static CANCELLATION_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static CANCELLATION_ROOT_STARTED: AtomicBool = AtomicBool::new(false);
static SCOPE_INTEGRATION_SINGLETON_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SCOPE_INTEGRATION_LOCAL_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static SCOPE_FAILURE_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static SCOPED_CONCURRENT_STATE: OnceLock<ScopedConcurrentState> = OnceLock::new();
static SCOPED_CANCELLATION_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static SCOPED_CANCELLATION_ROOT_STARTED: AtomicBool = AtomicBool::new(false);
static CLEANUP_TEST_LOCK: Mutex<()> = Mutex::new(());
static CLEANUP_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();

struct AsyncConcurrentState {
    started: AtomicUsize,
    events: Mutex<Vec<&'static str>>,
}

impl AsyncConcurrentState {
    fn reset(&self) {
        self.started.store(0, Ordering::SeqCst);
        self.events
            .lock()
            .expect("concurrent async state mutex should not be poisoned")
            .clear();
    }
}

fn async_concurrent_state() -> &'static AsyncConcurrentState {
    ASYNC_CONCURRENT_STATE.get_or_init(|| AsyncConcurrentState {
        started: AtomicUsize::new(0),
        events: Mutex::new(Vec::new()),
    })
}

struct ScopedConcurrentState {
    started: AtomicUsize,
    events: Mutex<Vec<&'static str>>,
}

impl ScopedConcurrentState {
    fn reset(&self) {
        self.started.store(0, Ordering::SeqCst);
        self.events
            .lock()
            .expect("scoped concurrent state mutex should not be poisoned")
            .clear();
    }
}

fn scoped_concurrent_state() -> &'static ScopedConcurrentState {
    SCOPED_CONCURRENT_STATE.get_or_init(|| ScopedConcurrentState {
        started: AtomicUsize::new(0),
        events: Mutex::new(Vec::new()),
    })
}

fn cleanup_events() -> &'static Mutex<Vec<&'static str>> {
    CLEANUP_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn reset_cleanup_events() {
    cleanup_events()
        .lock()
        .expect("cleanup event mutex should not be poisoned")
        .clear();
}

fn record_cleanup_event(event: &'static str) {
    cleanup_events()
        .lock()
        .expect("cleanup event mutex should not be poisoned")
        .push(event);
}

/// A test-only future that cannot complete until both independent factories have started.
///
/// A sequential activator would make the first factory remain pending forever. The bounded
/// hand-written driver below therefore turns accidental serialization into a deterministic test
/// failure instead of relying on a wall-clock timeout.
struct ConcurrentBarrierFuture {
    started: bool,
    start_event: &'static str,
    finish_event: &'static str,
    service: fn() -> ErasedService,
}

impl ConcurrentBarrierFuture {
    fn new(
        start_event: &'static str,
        finish_event: &'static str,
        service: fn() -> ErasedService,
    ) -> Self {
        Self {
            started: false,
            start_event,
            finish_event,
            service,
        }
    }
}

impl Future for ConcurrentBarrierFuture {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let state = async_concurrent_state();

        if !this.started {
            this.started = true;
            state
                .events
                .lock()
                .expect("concurrent async state mutex should not be poisoned")
                .push(this.start_event);
            state.started.fetch_add(1, Ordering::SeqCst);
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        if state.started.load(Ordering::SeqCst) != 2 {
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        state
            .events
            .lock()
            .expect("concurrent async state mutex should not be poisoned")
            .push(this.finish_event);
        Poll::Ready(Ok((this.service)()))
    }
}

/// The scoped counterpart to [`ConcurrentBarrierFuture`]. It proves that one scope's
/// independent async factories are progressed together, without relying on a timer.
struct ScopedConcurrentBarrierFuture {
    started: bool,
    start_event: &'static str,
    finish_event: &'static str,
    service: fn() -> ErasedService,
}

impl ScopedConcurrentBarrierFuture {
    fn new(
        start_event: &'static str,
        finish_event: &'static str,
        service: fn() -> ErasedService,
    ) -> Self {
        Self {
            started: false,
            start_event,
            finish_event,
            service,
        }
    }
}

impl Future for ScopedConcurrentBarrierFuture {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let state = scoped_concurrent_state();

        if !this.started {
            this.started = true;
            state
                .events
                .lock()
                .expect("scoped concurrent state mutex should not be poisoned")
                .push(this.start_event);
            state.started.fetch_add(1, Ordering::SeqCst);
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        if state.started.load(Ordering::SeqCst) != 2 {
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        state
            .events
            .lock()
            .expect("scoped concurrent state mutex should not be poisoned")
            .push(this.finish_event);
        Poll::Ready(Ok((this.service)()))
    }
}

/// A dependency factory that yields once before becoming constructible.
struct OrderedAsyncDependencyFuture {
    yielded: bool,
}

impl Future for OrderedAsyncDependencyFuture {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.yielded {
            this.yielded = true;
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        ORDERED_ASYNC_DEPENDENCY_READY.store(true, Ordering::SeqCst);
        Poll::Ready(Ok(ErasedService::new(OrderedAsyncDependency)))
    }
}

/// Two failures intentionally finish in the inverse of their compiled graph order.
///
/// The first provider waits for the second one to report failure. The activator must keep polling
/// all in-flight work and then report the graph-order-stable first failure.
struct StableFailureFuture {
    first: bool,
    started: bool,
}

impl StableFailureFuture {
    fn first() -> Self {
        Self {
            first: true,
            started: false,
        }
    }

    fn second() -> Self {
        Self {
            first: false,
            started: false,
        }
    }
}

impl Future for StableFailureFuture {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if !this.started {
            this.started = true;
            STABLE_FAILURE_STARTED.fetch_add(1, Ordering::SeqCst);
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        if STABLE_FAILURE_STARTED.load(Ordering::SeqCst) != 2 {
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        if this.first {
            if !STABLE_FAILURE_SECOND_RETURNED.load(Ordering::SeqCst) {
                context.waker().wake_by_ref();
                return Poll::Pending;
            }
            return Poll::Ready(Err(ActivationError::FactoryFailed {
                provider: "construct_stable_failure_first",
                provider_source: source(230),
            }));
        }

        STABLE_FAILURE_SECOND_RETURNED.store(true, Ordering::SeqCst);
        Poll::Ready(Err(ActivationError::FactoryFailed {
            provider: "construct_stable_failure_second",
            provider_source: source(231),
        }))
    }
}

/// A factory future that keeps a frame-bound dependency alive until cancellation.
struct CancellationPendingFuture<'frame> {
    _dependency: nestrs_core::__private::Inject<
        CancellationDependency,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
}

impl Future for CancellationPendingFuture<'_> {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

impl Drop for CancellationPendingFuture<'_> {
    fn drop(&mut self) {
        CANCELLATION_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("cancellation event mutex should not be poisoned")
            .push("factory future");
    }
}

/// Keeps both a parent Singleton token and a local Scoped token alive until cancellation.
struct ScopedCancellationPendingFuture<'frame> {
    _parent: nestrs_core::__private::Inject<
        ScopedCancellationParentSingleton,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
    _local: nestrs_core::__private::Inject<
        ScopedCancellationLocal,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
}

impl Future for ScopedCancellationPendingFuture<'_> {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

impl Drop for ScopedCancellationPendingFuture<'_> {
    fn drop(&mut self) {
        SCOPED_CANCELLATION_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("scoped cancellation event mutex should not be poisoned")
            .push("scoped factory future");
    }
}

/// A cleanup hook future that remains pending until the caller cancels shutdown.
///
/// It lets the test verify the key v3 invariant: dropping `shutdown()` still performs ordinary
/// Rust destruction, while the incomplete hook itself is not retried or completed implicitly.
struct ShutdownCleanupPendingFuture;

impl Future for ShutdownCleanupPendingFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

impl Drop for ShutdownCleanupPendingFuture {
    fn drop(&mut self) {
        record_cleanup_event("cancel root:cleanup future dropped");
    }
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the current-thread Tokio runtime should build for an async activation test")
        .block_on(future)
}

fn block_on_multi_thread<F>(future: F) -> F::Output
where
    F: Future,
{
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .expect("the multi-thread Tokio runtime should build for a parallel activation test")
        .block_on(future)
}

impl Drop for RollbackFirst {
    fn drop(&mut self) {
        ROLLBACK_DROP_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("rollback drop-order mutex should not be poisoned")
            .push("first");
    }
}

impl Drop for RollbackSecond {
    fn drop(&mut self) {
        ROLLBACK_DROP_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("rollback drop-order mutex should not be poisoned")
            .push("second");
    }
}

impl Drop for AsyncRollbackLeaf {
    fn drop(&mut self) {
        ASYNC_ROLLBACK_DROP_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("async rollback drop-order mutex should not be poisoned")
            .push("leaf");
    }
}

impl Drop for AsyncRollbackMiddle {
    fn drop(&mut self) {
        ASYNC_ROLLBACK_DROP_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("async rollback drop-order mutex should not be poisoned")
            .push("middle");
    }
}

impl Drop for CancellationDependency {
    fn drop(&mut self) {
        CANCELLATION_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("cancellation event mutex should not be poisoned")
            .push("dependency");
    }
}

impl Drop for ScopeFailureSingleton {
    fn drop(&mut self) {
        SCOPE_FAILURE_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("scope failure event mutex should not be poisoned")
            .push("singleton");
    }
}

impl Drop for ScopeFailureLocal {
    fn drop(&mut self) {
        SCOPE_FAILURE_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("scope failure event mutex should not be poisoned")
            .push("local");
    }
}

impl Drop for ScopedCancellationParentSingleton {
    fn drop(&mut self) {
        SCOPED_CANCELLATION_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("scoped cancellation event mutex should not be poisoned")
            .push("parent singleton");
    }
}

impl Drop for ScopedCancellationLocal {
    fn drop(&mut self) {
        SCOPED_CANCELLATION_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("scoped cancellation event mutex should not be poisoned")
            .push("scoped local");
    }
}

impl Drop for ShutdownDependency {
    fn drop(&mut self) {
        record_cleanup_event("singleton dependency:drop");
    }
}

impl Drop for ShutdownRoot {
    fn drop(&mut self) {
        record_cleanup_event("singleton root:drop");
    }
}

impl Drop for ShutdownScopeAppRoot {
    fn drop(&mut self) {
        record_cleanup_event("scope app:drop");
    }
}

impl Drop for ShutdownScopeLocal {
    fn drop(&mut self) {
        record_cleanup_event("scope local:drop");
    }
}

impl Drop for ShutdownScopeRoot {
    fn drop(&mut self) {
        record_cleanup_event("scope root:drop");
    }
}

impl Drop for ShutdownCancellationRoot {
    fn drop(&mut self) {
        record_cleanup_event("cancel root:drop");
    }
}

impl Drop for ShutdownAsyncFactoryRoot {
    fn drop(&mut self) {
        record_cleanup_event("async factory root:drop");
    }
}

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
    ServiceSource::new("tests/runtime.rs", line, 1)
}

fn singleton(line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime: Lifetime::Singleton,
        primary: false,
        source: source(line),
        cleanup: None,
    }
}

fn scoped(line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime: Lifetime::Scoped,
        ..singleton(line)
    }
}

fn construct_database(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(Database { label: "database" }))
}

fn construct_repository(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(Repository {
        database: context.take::<Database>(InputPosition(0))?,
    }))
}

fn construct_adapter(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(Adapter))
}

fn construct_produced<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> Result<ErasedService, ActivationError> {
    let database = context.take::<Database>(InputPosition(0))?;
    Ok(ErasedService::new(Produced {
        database_label: database.label,
    }))
}

fn construct_factory_root<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FactoryRoot {
        label: "factory root",
    }))
}

fn construct_keyed_adapter(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedAdapter))
}

fn construct_keyed_trait_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(KeyedTraitRoot {
        port: context.take::<dyn KeyedPort>(InputPosition(0))?,
    }))
}

fn construct_generic<T>(_context: ConstructionContext) -> Result<ErasedService, ActivationError>
where
    T: Injectable,
{
    Ok(ErasedService::new(Generic::<T> {
        label: "materialized generic",
        _marker: PhantomData,
    }))
}

fn generic_provider<T>() -> Provider
where
    T: Injectable,
{
    Provider::Class(ClassProvider {
        provide: identifier::<Generic<T>>(),
        common: singleton(108),
        dependencies: Vec::new(),
        constructor: construct_generic::<T>,
    })
}

fn materialize_generic_provider() -> Provider {
    GENERIC_MATERIALIZE_CALLS.fetch_add(1, Ordering::SeqCst);
    generic_provider::<GenericArgument>()
}

fn construct_generic_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(GenericRoot {
        first: context.take::<Generic<GenericArgument>>(InputPosition(0))?,
        second: context.take::<Generic<GenericArgument>>(InputPosition(1))?,
    }))
}

fn construct_runtime_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(RuntimeRoot {
        repository: context.take::<Repository>(InputPosition(0))?,
        produced: context.take::<Produced>(InputPosition(1))?,
        port: context.take::<dyn Port>(InputPosition(2))?,
        optional: context.take_optional::<OptionalService>(InputPosition(3))?,
    }))
}

fn construct_scoped_root(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedRoot))
}

fn construct_rollback_first(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(RollbackFirst))
}

fn construct_rollback_second(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(RollbackSecond))
}

fn construct_failing_root(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Err(ActivationError::FactoryFailed {
        provider: "construct_failing_root",
        provider_source: source(140),
    })
}

fn construct_async_factory_public_root<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async {
        Ok(ErasedService::new(AsyncFactoryPublicRoot {
            label: "async factory root",
        }))
    })
}

fn concurrent_left_service() -> ErasedService {
    ErasedService::new(ConcurrentAsyncLeft { label: "left" })
}

fn concurrent_right_service() -> ErasedService {
    ErasedService::new(ConcurrentAsyncRight { label: "right" })
}

fn construct_concurrent_async_left<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(ConcurrentBarrierFuture::new(
        "left:start",
        "left:finish",
        concurrent_left_service,
    ))
}

fn construct_concurrent_async_right<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(ConcurrentBarrierFuture::new(
        "right:start",
        "right:finish",
        concurrent_right_service,
    ))
}

fn construct_concurrent_async_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ConcurrentAsyncRoot {
        left: context.take::<ConcurrentAsyncLeft>(InputPosition(0))?,
        right: context.take::<ConcurrentAsyncRight>(InputPosition(1))?,
    }))
}

fn construct_ordered_async_dependency<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(OrderedAsyncDependencyFuture { yielded: false })
}

fn construct_ordered_async_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    let dependency = context.take::<OrderedAsyncDependency>(InputPosition(0));
    let started_early = !ORDERED_ASYNC_DEPENDENCY_READY.load(Ordering::SeqCst);
    ORDERED_ASYNC_ROOT_STARTED.store(true, Ordering::SeqCst);
    ORDERED_ASYNC_ROOT_STARTED_EARLY.store(started_early, Ordering::SeqCst);

    Box::pin(async move {
        let _dependency = dependency?;
        if started_early {
            return Err(ActivationError::FactoryFailed {
                provider: "construct_ordered_async_root",
                provider_source: source(211),
            });
        }
        Ok(ErasedService::new(OrderedAsyncRoot))
    })
}

fn construct_async_rollback_leaf(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncRollbackLeaf))
}

fn construct_async_rollback_middle(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncRollbackMiddle {
        _leaf: context.take::<AsyncRollbackLeaf>(InputPosition(0))?,
    }))
}

fn construct_async_rollback_failure<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async {
        Err(ActivationError::FactoryFailed {
            provider: "construct_async_rollback_failure",
            provider_source: source(221),
        })
    })
}

fn construct_stable_failure_first<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(StableFailureFuture::first())
}

fn construct_stable_failure_second<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(StableFailureFuture::second())
}

fn construct_stable_failure_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(StableFailureRoot))
}

fn construct_cancellation_dependency(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(CancellationDependency))
}

fn construct_cancellation_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    let dependency = match context.take::<CancellationDependency>(InputPosition(0)) {
        Ok(dependency) => dependency,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    CANCELLATION_ROOT_STARTED.store(true, Ordering::SeqCst);
    Box::pin(CancellationPendingFuture {
        _dependency: dependency,
    })
}

fn construct_scope_integration_app_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeIntegrationAppRoot {
        label: "scope app root",
    }))
}

fn construct_scope_integration_singleton(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeIntegrationSingleton {
        instance: SCOPE_INTEGRATION_SINGLETON_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_scope_integration_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeIntegrationLocal {
        instance: SCOPE_INTEGRATION_LOCAL_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }))
}

fn construct_scope_integration_request_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeIntegrationRequestRoot {
        singleton: context.take::<ScopeIntegrationSingleton>(InputPosition(0))?,
        local: context.take::<ScopeIntegrationLocal>(InputPosition(1))?,
    }))
}

fn construct_async_scope_integration_app_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncScopeIntegrationAppRoot))
}

fn construct_async_scope_integration_root<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async {
        Ok(ErasedService::new(AsyncScopeIntegrationRoot {
            label: "async scoped factory",
        }))
    })
}

fn construct_scope_failure_app_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeFailureAppRoot {
        singleton: context.take::<ScopeFailureSingleton>(InputPosition(0))?,
    }))
}

fn construct_scope_failure_singleton(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeFailureSingleton {
        label: "parent singleton",
    }))
}

fn construct_scope_failure_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeFailureLocal))
}

fn construct_scope_failure_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    let _singleton = context.take::<ScopeFailureSingleton>(InputPosition(0))?;
    let _local = context.take::<ScopeFailureLocal>(InputPosition(1))?;
    Err(ActivationError::FactoryFailed {
        provider: "construct_scope_failure_root",
        provider_source: source(330),
    })
}

fn construct_scoped_concurrent_app_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedConcurrentAppRoot))
}

fn scoped_concurrent_left_service() -> ErasedService {
    ErasedService::new(ScopedConcurrentLeft {
        label: "scoped left",
    })
}

fn scoped_concurrent_right_service() -> ErasedService {
    ErasedService::new(ScopedConcurrentRight {
        label: "scoped right",
    })
}

fn construct_scoped_concurrent_left<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(ScopedConcurrentBarrierFuture::new(
        "scoped left:start",
        "scoped left:finish",
        scoped_concurrent_left_service,
    ))
}

fn construct_scoped_concurrent_right<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(ScopedConcurrentBarrierFuture::new(
        "scoped right:start",
        "scoped right:finish",
        scoped_concurrent_right_service,
    ))
}

fn construct_scoped_concurrent_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedConcurrentRoot {
        left: context.take::<ScopedConcurrentLeft>(InputPosition(0))?,
        right: context.take::<ScopedConcurrentRight>(InputPosition(1))?,
    }))
}

fn construct_scoped_cancellation_parent_singleton(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedCancellationParentSingleton {
        label: "scoped cancellation parent",
    }))
}

fn construct_scoped_cancellation_app_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedCancellationAppRoot {
        parent: context.take::<ScopedCancellationParentSingleton>(InputPosition(0))?,
    }))
}

fn construct_scoped_cancellation_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopedCancellationLocal))
}

fn construct_scoped_cancellation_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    let parent = match context.take::<ScopedCancellationParentSingleton>(InputPosition(0)) {
        Ok(parent) => parent,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    let local = match context.take::<ScopedCancellationLocal>(InputPosition(1)) {
        Ok(local) => local,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    SCOPED_CANCELLATION_ROOT_STARTED.store(true, Ordering::SeqCst);
    Box::pin(ScopedCancellationPendingFuture {
        _parent: parent,
        _local: local,
    })
}

fn construct_shutdown_dependency(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownDependency))
}

fn construct_shutdown_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownRoot {
        _dependency: context.take::<ShutdownDependency>(InputPosition(0))?,
    }))
}

fn construct_shutdown_failure_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    let _dependency = context.take::<ShutdownDependency>(InputPosition(0))?;
    Err(ActivationError::FactoryFailed {
        provider: "construct_shutdown_failure_root",
        provider_source: source(400),
    })
}

fn construct_shutdown_scope_app_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownScopeAppRoot))
}

fn construct_shutdown_scope_local(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownScopeLocal))
}

fn construct_shutdown_scope_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownScopeRoot {
        _app: context.take::<ShutdownScopeAppRoot>(InputPosition(0))?,
        _local: context.take::<ShutdownScopeLocal>(InputPosition(1))?,
    }))
}

fn construct_shutdown_cancellation_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ShutdownCancellationRoot))
}

fn construct_shutdown_async_factory_root<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async { Ok(ErasedService::new(ShutdownAsyncFactoryRoot)) })
}

fn cleanup_shutdown_dependency() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("singleton dependency:cleanup");
    })
}

fn cleanup_shutdown_root() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("singleton root:cleanup");
    })
}

fn cleanup_shutdown_failure_root() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("failing root:cleanup");
    })
}

fn cleanup_shutdown_scope_app_root() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("scope app:cleanup");
    })
}

fn cleanup_shutdown_scope_local() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("scope local:cleanup");
    })
}

fn cleanup_shutdown_scope_root() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("scope root:cleanup");
    })
}

fn cleanup_shutdown_cancellation_root() -> CleanupFuture {
    record_cleanup_event("cancel root:cleanup started");
    Box::pin(ShutdownCleanupPendingFuture)
}

fn cleanup_shutdown_async_factory_root() -> CleanupFuture {
    Box::pin(async {
        record_cleanup_event("async factory root:cleanup");
    })
}

fn cleanup_cancellation_dependency() -> CleanupFuture {
    Box::pin(async {
        CANCELLATION_EVENTS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("cancellation event mutex should not be poisoned")
            .push("dependency cleanup");
    })
}

fn project_adapter(value: &Adapter) -> &(dyn Port + 'static) {
    value
}

fn prepare_adapter_required(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_required::<Adapter, dyn Port>(context, position, input, project_adapter)
}

fn prepare_adapter_optional(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_optional::<Adapter, dyn Port>(context, position, input, project_adapter)
}

fn project_keyed_adapter(value: &KeyedAdapter) -> &(dyn KeyedPort + 'static) {
    value
}

fn prepare_keyed_adapter_required(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_required::<KeyedAdapter, dyn KeyedPort>(
        context,
        position,
        input,
        project_keyed_adapter,
    )
}

fn prepare_keyed_adapter_optional(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError> {
    prepare_bound_optional::<KeyedAdapter, dyn KeyedPort>(
        context,
        position,
        input,
        project_keyed_adapter,
    )
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn database_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<Database>(),
        common: singleton(100),
        dependencies: Vec::new(),
        constructor: construct_database,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn repository_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<Repository>(),
        common: singleton(101),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<Database>(),
            optional: false,
            label: Some("database"),
            delivery: Delivery::Direct(prepare_required::<Database>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_repository,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn adapter_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<Adapter>(),
        common: singleton(102),
        dependencies: Vec::new(),
        constructor: construct_adapter,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn produced_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<Produced>(),
        common: singleton(103),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<Database>(),
            optional: false,
            label: Some("database"),
            delivery: Delivery::Direct(prepare_required::<Database>),
            provider_source: ProviderSource::Registered,
        }],
        invoker: FactoryInvoker::Sync(construct_produced),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn factory_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<FactoryRoot>(),
        common: singleton(105),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Sync(construct_factory_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_adapter_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: keyed_identifier::<KeyedAdapter>("keyed-port"),
        common: singleton(106),
        dependencies: Vec::new(),
        constructor: construct_keyed_adapter,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_trait_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<KeyedTraitRoot>(),
        common: singleton(107),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: keyed_identifier::<dyn KeyedPort>("keyed-port"),
            optional: false,
            label: Some("keyed port"),
            delivery: Delivery::RequiresBinding,
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_keyed_trait_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn runtime_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<RuntimeRoot>(),
        common: singleton(104),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<Repository>(),
                optional: false,
                label: Some("repository"),
                delivery: Delivery::Direct(prepare_required::<Repository>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<Produced>(),
                optional: false,
                label: Some("produced"),
                delivery: Delivery::Direct(prepare_required::<Produced>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 2,
                input_position: InputPosition(2),
                token: identifier::<dyn Port>(),
                optional: false,
                label: Some("port"),
                delivery: Delivery::RequiresBinding,
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 3,
                input_position: InputPosition(3),
                token: identifier::<OptionalService>(),
                optional: true,
                label: Some("optional"),
                delivery: Delivery::Direct(prepare_optional::<OptionalService>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_runtime_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn generic_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<GenericRoot>(),
        common: singleton(109),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<Generic<GenericArgument>>(),
                optional: false,
                label: Some("first generic"),
                delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                provider_source: ProviderSource::Materialize(materialize_generic_provider),
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<Generic<GenericArgument>>(),
                optional: false,
                label: Some("second generic"),
                delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                provider_source: ProviderSource::Materialize(materialize_generic_provider),
            },
        ],
        constructor: construct_generic_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedRoot>(),
        common: ProviderCommon {
            lifetime: Lifetime::Scoped,
            ..singleton(120)
        },
        dependencies: Vec::new(),
        constructor: construct_scoped_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn rollback_first_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<RollbackFirst>(),
        common: singleton(130),
        dependencies: Vec::new(),
        constructor: construct_rollback_first,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn rollback_second_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<RollbackSecond>(),
        common: singleton(131),
        dependencies: Vec::new(),
        constructor: construct_rollback_second,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failing_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailingRoot>(),
        common: singleton(132),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<RollbackFirst>(),
                optional: false,
                label: Some("first rollback dependency"),
                delivery: Delivery::Direct(prepare_required::<RollbackFirst>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<RollbackSecond>(),
                optional: false,
                label: Some("second rollback dependency"),
                delivery: Delivery::Direct(prepare_required::<RollbackSecond>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_failing_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_factory_public_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<AsyncFactoryPublicRoot>(),
        common: singleton(200),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_async_factory_public_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn concurrent_async_left_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<ConcurrentAsyncLeft>(),
        common: singleton(201),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_concurrent_async_left),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn concurrent_async_right_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<ConcurrentAsyncRight>(),
        common: singleton(202),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_concurrent_async_right),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn concurrent_async_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ConcurrentAsyncRoot>(),
        common: singleton(203),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ConcurrentAsyncLeft>(),
                optional: false,
                label: Some("left async dependency"),
                delivery: Delivery::Direct(prepare_required::<ConcurrentAsyncLeft>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ConcurrentAsyncRight>(),
                optional: false,
                label: Some("right async dependency"),
                delivery: Delivery::Direct(prepare_required::<ConcurrentAsyncRight>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_concurrent_async_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn ordered_async_dependency_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<OrderedAsyncDependency>(),
        common: singleton(210),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_ordered_async_dependency),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn ordered_async_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<OrderedAsyncRoot>(),
        common: singleton(211),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<OrderedAsyncDependency>(),
            optional: false,
            label: Some("ordered async dependency"),
            delivery: Delivery::Direct(prepare_required::<OrderedAsyncDependency>),
            provider_source: ProviderSource::Registered,
        }],
        invoker: FactoryInvoker::Async(construct_ordered_async_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_rollback_leaf_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncRollbackLeaf>(),
        common: singleton(220),
        dependencies: Vec::new(),
        constructor: construct_async_rollback_leaf,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_rollback_middle_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncRollbackMiddle>(),
        common: singleton(221),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<AsyncRollbackLeaf>(),
            optional: false,
            label: Some("async rollback leaf"),
            delivery: Delivery::Direct(prepare_required::<AsyncRollbackLeaf>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_async_rollback_middle,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_rollback_failure_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<AsyncRollbackFailureRoot>(),
        common: singleton(222),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<AsyncRollbackMiddle>(),
            optional: false,
            label: Some("async rollback middle"),
            delivery: Delivery::Direct(prepare_required::<AsyncRollbackMiddle>),
            provider_source: ProviderSource::Registered,
        }],
        invoker: FactoryInvoker::Async(construct_async_rollback_failure),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn stable_failure_first_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<StableFailureFirst>(),
        common: singleton(230),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_stable_failure_first),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn stable_failure_second_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<StableFailureSecond>(),
        common: singleton(231),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_stable_failure_second),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn stable_failure_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<StableFailureRoot>(),
        common: singleton(232),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<StableFailureFirst>(),
                optional: false,
                label: Some("first stable failure"),
                delivery: Delivery::Direct(prepare_required::<StableFailureFirst>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<StableFailureSecond>(),
                optional: false,
                label: Some("second stable failure"),
                delivery: Delivery::Direct(prepare_required::<StableFailureSecond>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_stable_failure_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_dependency_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<CancellationDependency>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_cancellation_dependency),
            ..singleton(240)
        },
        dependencies: Vec::new(),
        constructor: construct_cancellation_dependency,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<CancellationRoot>(),
        common: singleton(241),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<CancellationDependency>(),
            optional: false,
            label: Some("cancellation dependency"),
            delivery: Delivery::Direct(prepare_required::<CancellationDependency>),
            provider_source: ProviderSource::Registered,
        }],
        invoker: FactoryInvoker::Async(construct_cancellation_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_integration_app_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeIntegrationAppRoot>(),
        common: singleton(300),
        dependencies: Vec::new(),
        constructor: construct_scope_integration_app_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_integration_singleton_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeIntegrationSingleton>(),
        common: singleton(301),
        dependencies: Vec::new(),
        constructor: construct_scope_integration_singleton,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_integration_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeIntegrationLocal>(),
        common: scoped(302),
        dependencies: Vec::new(),
        constructor: construct_scope_integration_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_integration_request_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeIntegrationRequestRoot>(),
        common: scoped(303),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopeIntegrationSingleton>(),
                optional: false,
                label: Some("scope singleton"),
                delivery: Delivery::Direct(prepare_required::<ScopeIntegrationSingleton>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ScopeIntegrationLocal>(),
                optional: false,
                label: Some("scope local"),
                delivery: Delivery::Direct(prepare_required::<ScopeIntegrationLocal>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_scope_integration_request_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_scope_integration_app_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncScopeIntegrationAppRoot>(),
        common: singleton(310),
        dependencies: Vec::new(),
        constructor: construct_async_scope_integration_app_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_scope_integration_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<AsyncScopeIntegrationRoot>(),
        common: scoped(311),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_async_scope_integration_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_failure_singleton_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeFailureSingleton>(),
        common: singleton(320),
        dependencies: Vec::new(),
        constructor: construct_scope_failure_singleton,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_failure_app_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeFailureAppRoot>(),
        common: singleton(321),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<ScopeFailureSingleton>(),
            optional: false,
            label: Some("parent singleton"),
            delivery: Delivery::Direct(prepare_required::<ScopeFailureSingleton>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_scope_failure_app_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_failure_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeFailureLocal>(),
        common: scoped(322),
        dependencies: Vec::new(),
        constructor: construct_scope_failure_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_failure_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeFailureRoot>(),
        common: scoped(330),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopeFailureSingleton>(),
                optional: false,
                label: Some("scope failure singleton"),
                delivery: Delivery::Direct(prepare_required::<ScopeFailureSingleton>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ScopeFailureLocal>(),
                optional: false,
                label: Some("scope failure local"),
                delivery: Delivery::Direct(prepare_required::<ScopeFailureLocal>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_scope_failure_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_concurrent_app_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedConcurrentAppRoot>(),
        common: singleton(340),
        dependencies: Vec::new(),
        constructor: construct_scoped_concurrent_app_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_concurrent_left_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<ScopedConcurrentLeft>(),
        common: scoped(341),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_scoped_concurrent_left),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_concurrent_right_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<ScopedConcurrentRight>(),
        common: scoped(342),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_scoped_concurrent_right),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_concurrent_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedConcurrentRoot>(),
        common: scoped(343),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopedConcurrentLeft>(),
                optional: false,
                label: Some("scoped left async dependency"),
                delivery: Delivery::Direct(prepare_required::<ScopedConcurrentLeft>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ScopedConcurrentRight>(),
                optional: false,
                label: Some("scoped right async dependency"),
                delivery: Delivery::Direct(prepare_required::<ScopedConcurrentRight>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_scoped_concurrent_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_cancellation_parent_singleton_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedCancellationParentSingleton>(),
        common: singleton(350),
        dependencies: Vec::new(),
        constructor: construct_scoped_cancellation_parent_singleton,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_cancellation_app_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedCancellationAppRoot>(),
        common: singleton(351),
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<ScopedCancellationParentSingleton>(),
            optional: false,
            label: Some("scoped cancellation parent singleton"),
            delivery: Delivery::Direct(prepare_required::<ScopedCancellationParentSingleton>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_scoped_cancellation_app_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_cancellation_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopedCancellationLocal>(),
        common: scoped(352),
        dependencies: Vec::new(),
        constructor: construct_scoped_cancellation_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scoped_cancellation_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<ScopedCancellationRoot>(),
        common: scoped(353),
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopedCancellationParentSingleton>(),
                optional: false,
                label: Some("scoped cancellation parent singleton"),
                delivery: Delivery::Direct(prepare_required::<ScopedCancellationParentSingleton>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ScopedCancellationLocal>(),
                optional: false,
                label: Some("scoped cancellation local"),
                delivery: Delivery::Direct(prepare_required::<ScopedCancellationLocal>),
                provider_source: ProviderSource::Registered,
            },
        ],
        invoker: FactoryInvoker::Async(construct_scoped_cancellation_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_dependency_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownDependency>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_dependency),
            ..singleton(400)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_dependency,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_root),
            ..singleton(401)
        },
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<ShutdownDependency>(),
            optional: false,
            label: Some("shutdown dependency"),
            delivery: Delivery::Direct(prepare_required::<ShutdownDependency>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_shutdown_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_failure_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownFailureRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_failure_root),
            ..singleton(402)
        },
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_position: InputPosition(0),
            token: identifier::<ShutdownDependency>(),
            optional: false,
            label: Some("shutdown dependency"),
            delivery: Delivery::Direct(prepare_required::<ShutdownDependency>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_shutdown_failure_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_scope_app_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownScopeAppRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_scope_app_root),
            ..singleton(403)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_scope_app_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_scope_local_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownScopeLocal>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_scope_local),
            ..scoped(404)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_scope_local,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_scope_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownScopeRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_scope_root),
            ..scoped(405)
        },
        dependencies: vec![
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ShutdownScopeAppRoot>(),
                optional: false,
                label: Some("shutdown scope app root"),
                delivery: Delivery::Direct(prepare_required::<ShutdownScopeAppRoot>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 1,
                input_position: InputPosition(1),
                token: identifier::<ShutdownScopeLocal>(),
                optional: false,
                label: Some("shutdown scope local"),
                delivery: Delivery::Direct(prepare_required::<ShutdownScopeLocal>),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_shutdown_scope_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_cancellation_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ShutdownCancellationRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_cancellation_root),
            ..singleton(406)
        },
        dependencies: Vec::new(),
        constructor: construct_shutdown_cancellation_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn shutdown_async_factory_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<ShutdownAsyncFactoryRoot>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_shutdown_async_factory_root),
            ..singleton(407)
        },
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_shutdown_async_factory_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(::nestrs_core::__private::REFLECTED_BINDINGS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn port_binding() -> TraitBinding {
    TraitBinding {
        trait_type: ServiceType::create::<dyn Port>(),
        concrete_type: ServiceType::create::<Adapter>(),
        key_policy: BoundKeyPolicy::InheritRequestedKey,
        prepare_required: prepare_adapter_required,
        prepare_optional: prepare_adapter_optional,
        source: source(110),
    }
}

#[::nestrs_core::__private::linkme::distributed_slice(::nestrs_core::__private::REFLECTED_BINDINGS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn keyed_port_binding() -> TraitBinding {
    TraitBinding {
        trait_type: ServiceType::create::<dyn KeyedPort>(),
        concrete_type: ServiceType::create::<KeyedAdapter>(),
        key_policy: BoundKeyPolicy::InheritRequestedKey,
        prepare_required: prepare_keyed_adapter_required,
        prepare_optional: prepare_keyed_adapter_optional,
        source: source(111),
    }
}

#[test]
fn build_activates_a_root_with_class_factory_trait_and_optional_dependencies() {
    let provider = ServiceProvider::<RuntimeRoot>::build()
        .expect("the reachable singleton root graph should activate");
    let root = provider.root();

    assert_eq!(root.repository.database.label, "database");
    assert_eq!(root.produced.database_label, "database");
    assert_eq!(root.port.label(), "adapter");
    assert!(root.optional.is_none());
}

#[test]
fn build_activates_a_sync_factory_root_and_exposes_it_through_root() {
    let provider = ServiceProvider::<FactoryRoot>::build()
        .expect("a synchronous factory root should activate");

    assert_eq!(provider.root().label, "factory root");
}

#[test]
fn build_projects_a_keyed_trait_binding_through_the_public_root_api() {
    let provider = ServiceProvider::<KeyedTraitRoot>::build()
        .expect("a keyed trait binding should activate for the requested root");

    assert_eq!(provider.root().port.label(), "keyed adapter");
}

#[test]
fn build_materializes_a_closed_generic_dependency_for_the_public_root() {
    let _generic_test = GENERIC_MATERIALIZE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    GENERIC_MATERIALIZE_CALLS.store(0, Ordering::SeqCst);

    let provider = ServiceProvider::<GenericRoot>::build()
        .expect("a reachable closed generic dependency should materialize");
    let root = provider.root();

    assert_eq!(root.first.label, "materialized generic");
    assert_eq!(root.second.label, "materialized generic");
    assert_eq!(
        GENERIC_MATERIALIZE_CALLS.load(Ordering::SeqCst),
        1,
        "one closed generic token must materialize only once even when the root requests it twice"
    );
}

#[test]
fn build_async_preserves_mixed_sync_trait_key_and_optional_delivery() {
    let provider = block_on(ServiceProvider::<RuntimeRoot>::build_async())
        .expect("the async scheduler should also activate synchronous provider graphs");
    let root = provider.root();

    assert_eq!(root.repository.database.label, "database");
    assert_eq!(root.produced.database_label, "database");
    assert_eq!(root.port.label(), "adapter");
    assert!(root.optional.is_none());

    let keyed = block_on(ServiceProvider::<KeyedTraitRoot>::build_async())
        .expect("the async scheduler should preserve keyed trait projection");
    assert_eq!(keyed.root().port.label(), "keyed adapter");
}

#[test]
fn build_async_reports_when_no_tokio_runtime_is_active() {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut build = Box::pin(ServiceProvider::<RuntimeRoot>::build_async());

    let error = match build.as_mut().poll(&mut context) {
        Poll::Ready(Err(error)) => error,
        Poll::Ready(Ok(_)) => panic!("build_async must not activate without a Tokio runtime"),
        Poll::Pending => panic!("runtime validation must precede scheduling any worker"),
    };
    assert!(
        error.to_string().contains("活动 Tokio runtime"),
        "unexpected build error: {error}"
    );
}

#[test]
fn build_async_accepts_a_current_thread_tokio_runtime() {
    let provider = block_on(ServiceProvider::<AsyncFactoryPublicRoot>::build_async())
        .expect("a current-thread Tokio runtime should drive async activation safely");
    assert_eq!(provider.root().label, "async factory root");
}

#[test]
fn build_async_materializes_a_closed_generic_dependency_once() {
    let _generic_test = GENERIC_MATERIALIZE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    GENERIC_MATERIALIZE_CALLS.store(0, Ordering::SeqCst);

    let provider = block_on(ServiceProvider::<GenericRoot>::build_async())
        .expect("the async scheduler should materialize the reachable closed generic dependency");
    let root = provider.root();

    assert_eq!(root.first.label, "materialized generic");
    assert_eq!(root.second.label, "materialized generic");
    assert_eq!(
        GENERIC_MATERIALIZE_CALLS.load(Ordering::SeqCst),
        1,
        "one closed generic token must materialize once under build_async"
    );
}

#[test]
fn build_rejects_a_reachable_non_singleton_provider() {
    let error = match ServiceProvider::<ScopedRoot>::build() {
        Ok(_) => panic!("a reachable scoped provider must be rejected by v0"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("root 服务")
            && message.contains("Singleton")
            && message.contains("Scoped"),
        "unexpected build error: {message}"
    );
}

#[test]
fn build_async_keeps_rejecting_a_reachable_non_singleton_provider() {
    let error = match block_on(ServiceProvider::<ScopedRoot>::build_async()) {
        Ok(_) => panic!("build_async must not activate a reachable scoped provider"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("root 服务")
            && message.contains("Singleton")
            && message.contains("Scoped"),
        "unexpected build error: {message}"
    );
}

#[test]
fn build_rolls_back_committed_dependencies_in_reverse_order_when_root_activation_fails() {
    let drops = ROLLBACK_DROP_ORDER.get_or_init(|| Mutex::new(Vec::new()));
    drops
        .lock()
        .expect("rollback drop-order mutex should not be poisoned")
        .clear();

    let error = match ServiceProvider::<FailingRoot>::build() {
        Ok(_) => panic!("the intentionally failing root constructor must abort the build"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("激活 provider"),
        "unexpected build error: {error}"
    );
    assert_eq!(
        *drops
            .lock()
            .expect("rollback drop-order mutex should not be poisoned"),
        ["second", "first"],
        "dropping the failed build's Arena must destroy all committed dependencies in reverse order"
    );
}

#[test]
fn build_async_activates_an_async_factory_root_through_the_public_api() {
    let provider = block_on(ServiceProvider::<AsyncFactoryPublicRoot>::build_async())
        .expect("build_async should activate a reachable async factory");

    assert_eq!(provider.root().label, "async factory root");
}

#[test]
fn build_async_overlaps_independent_factory_futures_without_a_timing_assumption() {
    let state = async_concurrent_state();
    state.reset();

    let provider = block_on_multi_thread(ServiceProvider::<ConcurrentAsyncRoot>::build_async())
        .expect("independent async factories should be progressed together");
    let root = provider.root();

    assert_eq!(root.left.label, "left");
    assert_eq!(root.right.label, "right");

    let events = state
        .events
        .lock()
        .expect("concurrent async state mutex should not be poisoned")
        .clone();
    let first_finish = events
        .iter()
        .position(|event| event.ends_with(":finish"))
        .expect("both barrier futures should complete");

    assert_eq!(
        state.started.load(Ordering::SeqCst),
        2,
        "both independent factories must have started"
    );
    assert_eq!(
        first_finish, 2,
        "neither factory may finish before both independent factories have entered Pending"
    );
    assert!(
        events[..first_finish]
            .iter()
            .all(|event| event.ends_with(":start")),
        "the completion barrier must be structural rather than time based"
    );
}

#[test]
fn build_async_does_not_start_a_consumer_before_its_dependency_is_committed() {
    ORDERED_ASYNC_DEPENDENCY_READY.store(false, Ordering::SeqCst);
    ORDERED_ASYNC_ROOT_STARTED.store(false, Ordering::SeqCst);
    ORDERED_ASYNC_ROOT_STARTED_EARLY.store(false, Ordering::SeqCst);

    block_on(ServiceProvider::<OrderedAsyncRoot>::build_async())
        .expect("a consumer should only start after its async dependency succeeds");

    assert!(
        ORDERED_ASYNC_DEPENDENCY_READY.load(Ordering::SeqCst),
        "the dependency factory must have completed"
    );
    assert!(
        ORDERED_ASYNC_ROOT_STARTED.load(Ordering::SeqCst),
        "the consumer factory should eventually run"
    );
    assert!(
        !ORDERED_ASYNC_ROOT_STARTED_EARLY.load(Ordering::SeqCst),
        "the consumer factory must not start while its dependency is still pending"
    );
}

#[test]
fn build_keeps_rejecting_a_reachable_async_factory() {
    let error = match ServiceProvider::<AsyncFactoryPublicRoot>::build() {
        Ok(_) => panic!("the synchronous build API must not activate async factories"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("使用了 async factory"),
        "unexpected build error: {error}"
    );
}

#[test]
fn build_async_rolls_back_committed_ancestors_when_an_async_root_fails() {
    let drops = ASYNC_ROLLBACK_DROP_ORDER.get_or_init(|| Mutex::new(Vec::new()));
    drops
        .lock()
        .expect("async rollback drop-order mutex should not be poisoned")
        .clear();

    let error = match block_on(ServiceProvider::<AsyncRollbackFailureRoot>::build_async()) {
        Ok(_) => panic!("the intentionally failing async root must abort the build"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("激活 provider") && message.contains(&format!("{:?}", source(222))),
        "unexpected build error: {message}"
    );
    assert_eq!(
        *drops
            .lock()
            .expect("async rollback drop-order mutex should not be poisoned"),
        ["middle", "leaf"],
        "the failed async build must roll back committed ancestors in reverse dependency order"
    );
}

#[test]
fn build_async_waits_for_in_flight_failures_and_reports_graph_order_stably() {
    STABLE_FAILURE_STARTED.store(0, Ordering::SeqCst);
    STABLE_FAILURE_SECOND_RETURNED.store(false, Ordering::SeqCst);

    let error = match block_on(ServiceProvider::<StableFailureRoot>::build_async()) {
        Ok(_) => panic!("both independent factories intentionally fail"),
        Err(error) => error,
    };

    assert_eq!(
        STABLE_FAILURE_STARTED.load(Ordering::SeqCst),
        2,
        "the failure barrier proves both sibling factories were in flight"
    );
    assert!(
        STABLE_FAILURE_SECOND_RETURNED.load(Ordering::SeqCst),
        "the second provider must fail before the first provider is allowed to fail"
    );
    let message = error.to_string();
    assert!(
        message.contains("激活 provider") && message.contains(&format!("{:?}", source(230))),
        "unexpected build error: {message}"
    );
}

#[test]
fn dropping_build_async_drops_in_flight_factory_before_rolling_back_the_arena() {
    let events = CANCELLATION_EVENTS.get_or_init(|| Mutex::new(Vec::new()));
    events
        .lock()
        .expect("cancellation event mutex should not be poisoned")
        .clear();
    CANCELLATION_ROOT_STARTED.store(false, Ordering::SeqCst);

    block_on(async {
        let mut build = Box::pin(ServiceProvider::<CancellationRoot>::build_async());

        for _ in 0..128 {
            std::future::poll_fn(|context| match build.as_mut().poll(context) {
                Poll::Ready(_) => {
                    panic!("the cancellation fixture's factory must remain pending")
                }
                Poll::Pending => Poll::Ready(()),
            })
            .await;
            if CANCELLATION_ROOT_STARTED.load(Ordering::SeqCst) {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert!(
            CANCELLATION_ROOT_STARTED.load(Ordering::SeqCst),
            "the pending factory must be in flight before build_async is cancelled"
        );
        drop(build);

        for _ in 0..128 {
            if *events
                .lock()
                .expect("cancellation event mutex should not be poisoned")
                == ["factory future", "dependency"]
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("Tokio abort should eventually drop the factory frame before its Arena backing");
    });

    assert_eq!(
        *events
            .lock()
            .expect("cancellation event mutex should not be poisoned"),
        ["factory future", "dependency"],
        "cancellation must drop the in-flight factory before the Arena releases its dependency"
    );
}

#[test]
fn scope_provider_shares_singletons_and_rebuilds_scoped_dependencies_per_scope() {
    SCOPE_INTEGRATION_SINGLETON_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    SCOPE_INTEGRATION_LOCAL_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let provider =
        ScopeProvider::<ScopeIntegrationAppRoot, ScopeLayer<ScopeIntegrationRequestRoot>>::build()
            .expect("the singleton application and scoped request graph should compile");
    let first_scope = provider
        .create_scope()
        .expect("the first scoped request should activate");
    let second_scope = provider
        .create_scope()
        .expect("the second scoped request should activate independently");
    let first_root = first_scope.root();
    let second_root = second_scope.root();

    assert_eq!(provider.root().label, "scope app root");
    assert_eq!(
        first_root.singleton.instance, second_root.singleton.instance,
        "each scope must receive the same parent singleton instance"
    );
    assert_ne!(
        first_root.local.instance, second_root.local.instance,
        "each scope must receive its own scoped instance"
    );
    assert_eq!(
        SCOPE_INTEGRATION_SINGLETON_CONSTRUCTIONS.load(Ordering::SeqCst),
        1,
        "the singleton reachable only through the scope root should still activate once in the parent"
    );
    assert_eq!(
        SCOPE_INTEGRATION_LOCAL_CONSTRUCTIONS.load(Ordering::SeqCst),
        2,
        "one scoped local instance should be built for each scope"
    );
}

#[test]
fn scope_provider_build_async_and_create_scope_async_activate_an_async_scoped_factory() {
    let provider = block_on(ScopeProvider::<
        AsyncScopeIntegrationAppRoot,
        ScopeLayer<AsyncScopeIntegrationRoot>,
    >::build_async())
    .expect("the async scoped graph should compile and activate its singleton application root");
    let scope = block_on(provider.create_scope_async())
        .expect("create_scope_async should activate a reachable async scoped factory");

    assert_eq!(scope.root().label, "async scoped factory");
}

#[test]
fn scope_provider_build_rejects_a_reachable_async_scoped_factory() {
    let error = match ScopeProvider::<
        AsyncScopeIntegrationAppRoot,
        ScopeLayer<AsyncScopeIntegrationRoot>,
    >::build()
    {
        Ok(_) => panic!("the synchronous scope API must reject async scoped factories"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("使用了 async factory"),
        "unexpected build error: {error}"
    );
}

#[test]
fn failed_scope_rolls_back_its_local_arena_without_releasing_parent_singletons() {
    let events = SCOPE_FAILURE_EVENTS.get_or_init(|| Mutex::new(Vec::new()));
    events
        .lock()
        .expect("scope failure event mutex should not be poisoned")
        .clear();

    let provider = ScopeProvider::<ScopeFailureAppRoot, ScopeLayer<ScopeFailureRoot>>::build()
        .expect("the parent singleton graph should activate before creating a scope");
    let error = match provider.create_scope() {
        Ok(_) => panic!("the intentionally failing scoped root must abort only its own scope"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("激活 provider") && message.contains(&format!("{:?}", source(330))),
        "unexpected build error: {message}"
    );
    assert_eq!(
        provider.root().singleton.label,
        "parent singleton",
        "a failed child scope must leave its parent Singleton Arena usable"
    );
    assert_eq!(
        *events
            .lock()
            .expect("scope failure event mutex should not be poisoned"),
        ["local"],
        "only the local scoped instance should roll back while the provider remains alive"
    );
}

#[test]
fn create_scope_async_overlaps_independent_scoped_factory_futures_without_timing() {
    let state = scoped_concurrent_state();
    state.reset();

    let provider = block_on(ScopeProvider::<
        ScopedConcurrentAppRoot,
        ScopeLayer<ScopedConcurrentRoot>,
    >::build_async())
    .expect("the scoped concurrent graph should build its parent singleton root");
    let scope = block_on(provider.create_scope_async())
        .expect("independent scoped factory futures should be progressed together");
    let root = scope.root();

    assert_eq!(root.left.label, "scoped left");
    assert_eq!(root.right.label, "scoped right");

    let events = state
        .events
        .lock()
        .expect("scoped concurrent state mutex should not be poisoned")
        .clone();
    let first_finish = events
        .iter()
        .position(|event| event.ends_with(":finish"))
        .expect("both scoped barrier futures should complete");

    assert_eq!(
        state.started.load(Ordering::SeqCst),
        2,
        "both independent scoped factories must have started"
    );
    assert_eq!(
        first_finish, 2,
        "neither scoped factory may finish before both independent factories have entered Pending"
    );
    assert!(
        events[..first_finish]
            .iter()
            .all(|event| event.ends_with(":start")),
        "the scoped completion barrier must be structural rather than time based"
    );
}

#[test]
fn dropping_create_scope_async_drops_in_flight_work_then_rolls_back_only_the_scoped_arena() {
    let events = SCOPED_CANCELLATION_EVENTS.get_or_init(|| Mutex::new(Vec::new()));
    events
        .lock()
        .expect("scoped cancellation event mutex should not be poisoned")
        .clear();
    SCOPED_CANCELLATION_ROOT_STARTED.store(false, Ordering::SeqCst);

    let provider = block_on(ScopeProvider::<
        ScopedCancellationAppRoot,
        ScopeLayer<ScopedCancellationRoot>,
    >::build_async())
    .expect("the parent singleton graph should activate before a scoped async creation starts");
    block_on(async {
        let mut creation = Box::pin(provider.create_scope_async());

        for _ in 0..128 {
            std::future::poll_fn(|context| match creation.as_mut().poll(context) {
                Poll::Ready(_) => {
                    panic!("the scoped cancellation fixture's factory must remain pending")
                }
                Poll::Pending => Poll::Ready(()),
            })
            .await;
            if SCOPED_CANCELLATION_ROOT_STARTED.load(Ordering::SeqCst) {
                break;
            }
            tokio::task::yield_now().await;
        }

        assert!(
            SCOPED_CANCELLATION_ROOT_STARTED.load(Ordering::SeqCst),
            "the pending scoped factory must be in flight before create_scope_async is cancelled"
        );
        drop(creation);

        for _ in 0..128 {
            if *events
                .lock()
                .expect("scoped cancellation event mutex should not be poisoned")
                == ["scoped factory future", "scoped local"]
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("Tokio abort should eventually roll back the local Scoped Arena");
    });

    assert_eq!(
        provider.root().parent.label,
        "scoped cancellation parent",
        "cancelling a child scope must leave its parent singleton graph usable"
    );
    assert_eq!(
        *events
            .lock()
            .expect("scoped cancellation event mutex should not be poisoned"),
        ["scoped factory future", "scoped local"],
        "cancellation must drop the in-flight scoped factory before rolling back only local services"
    );
}

#[test]
fn sync_build_rejects_cleanup_but_async_shutdown_runs_hooks_before_reverse_drops() {
    let _cleanup_guard = CLEANUP_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_cleanup_events();

    let error = match ServiceProvider::<ShutdownRoot>::build() {
        Ok(_) => panic!("the synchronous build API must reject reachable cleanup hooks"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("cleanup hook"),
        "unexpected build error: {error}"
    );

    let provider = block_on(ServiceProvider::<ShutdownRoot>::build_async())
        .expect("build_async should accept and retain reachable cleanup hooks");
    assert_eq!(
        std::mem::size_of_val(&*provider.root()._dependency),
        0,
        "the public root should remain usable before consuming shutdown"
    );

    block_on(provider.shutdown());

    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        [
            "singleton root:cleanup",
            "singleton root:drop",
            "singleton dependency:cleanup",
            "singleton dependency:drop",
        ],
        "explicit shutdown must run each hook immediately before dropping its service in reverse commit order"
    );

    reset_cleanup_events();
    let provider = block_on(ServiceProvider::<ShutdownAsyncFactoryRoot>::build_async())
        .expect("an async factory with cleanup should activate through build_async");
    block_on(provider.shutdown());
    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        ["async factory root:cleanup", "async factory root:drop"],
        "the async factory activation path must preserve cleanup metadata through commit"
    );
}

#[test]
fn ordinary_drop_and_failed_async_build_do_not_drive_cleanup_hooks() {
    let _cleanup_guard = CLEANUP_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    reset_cleanup_events();
    {
        let provider = block_on(ServiceProvider::<ShutdownRoot>::build_async())
            .expect("the cleanup fixture should build asynchronously");
        drop(provider);
    }
    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        ["singleton root:drop", "singleton dependency:drop"],
        "ordinary Drop must retain Rust reverse destruction without starting async cleanup"
    );

    reset_cleanup_events();
    let error = match block_on(ServiceProvider::<ShutdownFailureRoot>::build_async()) {
        Ok(_) => panic!("the cleanup failure fixture must fail during root activation"),
        Err(error) => error,
    };
    let message = error.to_string();
    assert!(
        message.contains("激活 provider") && message.contains(&format!("{:?}", source(402))),
        "unexpected build error: {message}"
    );
    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        ["singleton dependency:drop"],
        "activation failure must roll back committed services with Rust Drop only"
    );
}

#[test]
fn scope_shutdown_isolated_from_parent_and_sync_scope_paths_reject_cleanup() {
    let _cleanup_guard = CLEANUP_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());

    reset_cleanup_events();
    let error = match ScopeProvider::<ShutdownScopeAppRoot, ScopeLayer<ShutdownScopeRoot>>::build()
    {
        Ok(_) => panic!("the synchronous scope builder must reject reachable cleanup hooks"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("cleanup hook"),
        "unexpected build error: {error}"
    );

    let provider = block_on(ScopeProvider::<
        ShutdownScopeAppRoot,
        ScopeLayer<ShutdownScopeRoot>,
    >::build_async())
    .expect("the async scope builder should accept parent and scoped cleanup hooks");
    let error = match provider.create_scope() {
        Ok(_) => panic!("the synchronous scope creation API must reject scoped cleanup hooks"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("cleanup hook"),
        "unexpected build error: {error}"
    );
    drop(provider);
    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        ["scope app:drop"],
        "dropping an async-built provider after a rejected sync scope must not run its parent hook"
    );

    reset_cleanup_events();
    let provider = block_on(ScopeProvider::<
        ShutdownScopeAppRoot,
        ScopeLayer<ShutdownScopeRoot>,
    >::build_async())
    .expect("the async scoped cleanup fixture should build");
    let scope = block_on(provider.create_scope_async())
        .expect("create_scope_async should accept scoped cleanup hooks");
    let _scope_root = scope.root();
    block_on(scope.shutdown());

    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        [
            "scope root:cleanup",
            "scope root:drop",
            "scope local:cleanup",
            "scope local:drop",
        ],
        "scope shutdown must only consume its own Scoped Arena"
    );
    let _parent_root = provider.root();
    block_on(provider.shutdown());
    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        [
            "scope root:cleanup",
            "scope root:drop",
            "scope local:cleanup",
            "scope local:drop",
            "scope app:cleanup",
            "scope app:drop",
        ],
        "the parent Singleton hook must wait for explicit provider shutdown"
    );
}

#[test]
fn cancelling_shutdown_drops_services_without_completing_the_pending_hook() {
    let _cleanup_guard = CLEANUP_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    reset_cleanup_events();

    let provider = block_on(ServiceProvider::<ShutdownCancellationRoot>::build_async())
        .expect("the cancellation fixture should build asynchronously");
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut shutdown = Box::pin(provider.shutdown());

    assert!(matches!(
        shutdown.as_mut().poll(&mut context),
        Poll::Pending
    ));
    drop(shutdown);

    assert_eq!(
        *cleanup_events()
            .lock()
            .expect("cleanup event mutex should not be poisoned"),
        [
            "cancel root:cleanup started",
            "cancel root:cleanup future dropped",
            "cancel root:drop",
        ],
        "cancelling shutdown must drop the in-flight hook and still safely destruct the Arena"
    );
}
