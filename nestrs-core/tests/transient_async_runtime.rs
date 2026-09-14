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
    ServiceProvider,
    scope::{ScopeLayer, ScopeProvider},
};

type Inject<T> = nestrs_core::__private::Inject<T>;

struct AsyncTransientLeft {
    label: &'static str,
}

struct AsyncTransientRight {
    label: &'static str,
}

struct AsyncTransientOverlapRoot {
    left: Inject<AsyncTransientLeft>,
    right: Inject<AsyncTransientRight>,
}

struct FailureTransient;

struct FailureOwner {
    _transient: Inject<FailureTransient>,
}

struct FailureRoot;

struct CancellationTransient;

struct CancellationOwner {
    _transient: Inject<CancellationTransient>,
}

struct CancellationRoot;

struct TemporaryParameterTransient;

struct TemporaryParameterRoot;

struct ScopeShutdownParentTransient;

struct ScopeShutdownApp {
    _transient: Inject<ScopeShutdownParentTransient>,
}

struct ScopeShutdownLocalTransient;

struct ScopeShutdownRoot {
    _transient: Inject<ScopeShutdownLocalTransient>,
}

static ASYNC_TRANSIENT_OVERLAP: OnceLock<AsyncTransientOverlapState> = OnceLock::new();
static FAILURE_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static CANCELLATION_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static CANCELLATION_ROOT_STARTED: AtomicBool = AtomicBool::new(false);
static TEMPORARY_PARAMETER_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static SCOPE_SHUTDOWN_EVENTS: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
static SCOPE_SHUTDOWN_TEST_LOCK: Mutex<()> = Mutex::new(());

struct AsyncTransientOverlapState {
    started: AtomicUsize,
    events: Mutex<Vec<&'static str>>,
}

impl AsyncTransientOverlapState {
    fn reset(&self) {
        self.started.store(0, Ordering::SeqCst);
        self.events
            .lock()
            .expect("async transient overlap mutex should not be poisoned")
            .clear();
    }
}

fn async_transient_overlap_state() -> &'static AsyncTransientOverlapState {
    ASYNC_TRANSIENT_OVERLAP.get_or_init(|| AsyncTransientOverlapState {
        started: AtomicUsize::new(0),
        events: Mutex::new(Vec::new()),
    })
}

fn failure_events() -> &'static Mutex<Vec<&'static str>> {
    FAILURE_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_failure(event: &'static str) {
    failure_events()
        .lock()
        .expect("failure rollback mutex should not be poisoned")
        .push(event);
}

fn cancellation_events() -> &'static Mutex<Vec<&'static str>> {
    CANCELLATION_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_cancellation(event: &'static str) {
    cancellation_events()
        .lock()
        .expect("cancellation rollback mutex should not be poisoned")
        .push(event);
}

fn temporary_parameter_events() -> &'static Mutex<Vec<&'static str>> {
    TEMPORARY_PARAMETER_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_temporary_parameter(event: &'static str) {
    temporary_parameter_events()
        .lock()
        .expect("temporary parameter mutex should not be poisoned")
        .push(event);
}

fn scope_shutdown_events() -> &'static Mutex<Vec<&'static str>> {
    SCOPE_SHUTDOWN_EVENTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_scope_shutdown(event: &'static str) {
    scope_shutdown_events()
        .lock()
        .expect("scope shutdown mutex should not be poisoned")
        .push(event);
}

/// A structural barrier: it can complete only after both independent transient factories have
/// entered `Pending`. A sequential activation loop would therefore never finish this test.
struct AsyncTransientBarrierFuture {
    started: bool,
    start_event: &'static str,
    finish_event: &'static str,
    service: fn() -> ErasedService,
}

impl AsyncTransientBarrierFuture {
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

impl Future for AsyncTransientBarrierFuture {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let state = async_transient_overlap_state();

        if !this.started {
            this.started = true;
            state
                .events
                .lock()
                .expect("async transient overlap mutex should not be poisoned")
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
            .expect("async transient overlap mutex should not be poisoned")
            .push(this.finish_event);
        Poll::Ready(Ok((this.service)()))
    }
}

/// This future retains the factory parameter token while it is pending. Dropping the build
/// future must discard it before rolling back the owner and its transient child arena.
struct CancellationRootFuture<'frame> {
    _owner: nestrs_core::__private::Inject<
        CancellationOwner,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
}

impl Future for CancellationRootFuture<'_> {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        context.waker().wake_by_ref();
        Poll::Pending
    }
}

impl Drop for CancellationRootFuture<'_> {
    fn drop(&mut self) {
        record_cancellation("root factory future");
    }
}

/// A completed factory future must be released before its transient parameter's temporary arena.
struct TemporaryParameterRootFuture<'frame> {
    _transient: nestrs_core::__private::Inject<
        TemporaryParameterTransient,
        nestrs_core::__private::FactoryParameter<'frame>,
    >,
}

impl Future for TemporaryParameterRootFuture<'_> {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Ready(Ok(ErasedService::new(TemporaryParameterRoot)))
    }
}

impl Drop for TemporaryParameterRootFuture<'_> {
    fn drop(&mut self) {
        record_temporary_parameter("factory future");
    }
}

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
    ServiceSource::new("tests/transient_async_runtime.rs", line, 1)
}

fn common(lifetime: Lifetime, line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime,
        primary: false,
        source: source(line),
        cleanup: None,
    }
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

impl Drop for FailureTransient {
    fn drop(&mut self) {
        record_failure("transient");
    }
}

impl Drop for FailureOwner {
    fn drop(&mut self) {
        record_failure("owner");
    }
}

impl Drop for CancellationTransient {
    fn drop(&mut self) {
        record_cancellation("transient");
    }
}

impl Drop for CancellationOwner {
    fn drop(&mut self) {
        record_cancellation("owner");
    }
}

impl Drop for TemporaryParameterTransient {
    fn drop(&mut self) {
        record_temporary_parameter("transient");
    }
}

impl Drop for ScopeShutdownParentTransient {
    fn drop(&mut self) {
        record_scope_shutdown("parent transient:drop");
    }
}

impl Drop for ScopeShutdownApp {
    fn drop(&mut self) {
        record_scope_shutdown("parent root:drop");
    }
}

impl Drop for ScopeShutdownLocalTransient {
    fn drop(&mut self) {
        record_scope_shutdown("scope transient:drop");
    }
}

impl Drop for ScopeShutdownRoot {
    fn drop(&mut self) {
        record_scope_shutdown("scope root:drop");
    }
}

fn construct_async_transient_left<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(AsyncTransientBarrierFuture::new(
        "left:start",
        "left:finish",
        || {
            ErasedService::new(AsyncTransientLeft {
                label: "left transient",
            })
        },
    ))
}

fn construct_async_transient_right<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(AsyncTransientBarrierFuture::new(
        "right:start",
        "right:finish",
        || {
            ErasedService::new(AsyncTransientRight {
                label: "right transient",
            })
        },
    ))
}

fn construct_async_transient_overlap_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(AsyncTransientOverlapRoot {
        left: context.take::<AsyncTransientLeft>(InputPosition(0))?,
        right: context.take::<AsyncTransientRight>(InputPosition(1))?,
    }))
}

fn construct_failure_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FailureTransient))
}

fn construct_failure_owner(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(FailureOwner {
        _transient: context.take::<FailureTransient>(InputPosition(0))?,
    }))
}

fn construct_failure_root<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(async {
        Err(ActivationError::FactoryFailed {
            provider: "construct_failure_root",
            provider_source: source(220),
        })
    })
}

fn construct_cancellation_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(CancellationTransient))
}

fn construct_cancellation_owner(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(CancellationOwner {
        _transient: context.take::<CancellationTransient>(InputPosition(0))?,
    }))
}

fn construct_cancellation_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    let owner = match context.take::<CancellationOwner>(InputPosition(0)) {
        Ok(owner) => owner,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    CANCELLATION_ROOT_STARTED.store(true, Ordering::SeqCst);
    Box::pin(CancellationRootFuture { _owner: owner })
}

fn construct_temporary_parameter_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(TemporaryParameterTransient))
}

fn construct_temporary_parameter_root<'frame>(
    mut context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    let transient = match context.take::<TemporaryParameterTransient>(InputPosition(0)) {
        Ok(transient) => transient,
        Err(error) => return Box::pin(async move { Err(error) }),
    };
    Box::pin(TemporaryParameterRootFuture {
        _transient: transient,
    })
}

fn construct_scope_shutdown_parent_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeShutdownParentTransient))
}

fn construct_scope_shutdown_app(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeShutdownApp {
        _transient: context.take::<ScopeShutdownParentTransient>(InputPosition(0))?,
    }))
}

fn construct_scope_shutdown_local_transient(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeShutdownLocalTransient))
}

fn construct_scope_shutdown_root(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(ScopeShutdownRoot {
        _transient: context.take::<ScopeShutdownLocalTransient>(InputPosition(0))?,
    }))
}

fn cleanup_scope_shutdown_parent_transient() -> CleanupFuture {
    Box::pin(async { record_scope_shutdown("parent transient:cleanup") })
}

fn cleanup_scope_shutdown_local_transient() -> CleanupFuture {
    Box::pin(async { record_scope_shutdown("scope transient:cleanup") })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_transient_left_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<AsyncTransientLeft>(),
        common: common(Lifetime::Transient, 300),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_async_transient_left),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_transient_right_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<AsyncTransientRight>(),
        common: common(Lifetime::Transient, 310),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_async_transient_right),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn async_transient_overlap_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<AsyncTransientOverlapRoot>(),
        common: common(Lifetime::Singleton, 320),
        dependencies: vec![
            dependency::<AsyncTransientLeft>(0),
            dependency::<AsyncTransientRight>(1),
        ],
        constructor: construct_async_transient_overlap_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureTransient>(),
        common: common(Lifetime::Transient, 330),
        dependencies: Vec::new(),
        constructor: construct_failure_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_owner_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<FailureOwner>(),
        common: common(Lifetime::Singleton, 340),
        dependencies: vec![dependency::<FailureTransient>(0)],
        constructor: construct_failure_owner,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn failure_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<FailureRoot>(),
        common: common(Lifetime::Singleton, 350),
        dependencies: vec![dependency::<FailureOwner>(0)],
        invoker: FactoryInvoker::Async(construct_failure_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<CancellationTransient>(),
        common: common(Lifetime::Transient, 360),
        dependencies: Vec::new(),
        constructor: construct_cancellation_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_owner_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<CancellationOwner>(),
        common: common(Lifetime::Singleton, 370),
        dependencies: vec![dependency::<CancellationTransient>(0)],
        constructor: construct_cancellation_owner,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn cancellation_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<CancellationRoot>(),
        common: common(Lifetime::Singleton, 380),
        dependencies: vec![dependency::<CancellationOwner>(0)],
        invoker: FactoryInvoker::Async(construct_cancellation_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn temporary_parameter_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<TemporaryParameterTransient>(),
        common: common(Lifetime::Transient, 385),
        dependencies: Vec::new(),
        constructor: construct_temporary_parameter_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn temporary_parameter_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<TemporaryParameterRoot>(),
        common: common(Lifetime::Singleton, 387),
        dependencies: vec![dependency::<TemporaryParameterTransient>(0)],
        invoker: FactoryInvoker::Async(construct_temporary_parameter_root),
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_shutdown_parent_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeShutdownParentTransient>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_scope_shutdown_parent_transient),
            ..common(Lifetime::Transient, 390)
        },
        dependencies: Vec::new(),
        constructor: construct_scope_shutdown_parent_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_shutdown_app_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeShutdownApp>(),
        common: common(Lifetime::Singleton, 400),
        dependencies: vec![dependency::<ScopeShutdownParentTransient>(0)],
        constructor: construct_scope_shutdown_app,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_shutdown_local_transient_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeShutdownLocalTransient>(),
        common: ProviderCommon {
            cleanup: Some(cleanup_scope_shutdown_local_transient),
            ..common(Lifetime::Transient, 410)
        },
        dependencies: Vec::new(),
        constructor: construct_scope_shutdown_local_transient,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(REFLECTED_PROVIDERS)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn scope_shutdown_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<ScopeShutdownRoot>(),
        common: common(Lifetime::Scoped, 420),
        dependencies: vec![dependency::<ScopeShutdownLocalTransient>(0)],
        constructor: construct_scope_shutdown_root,
    })
}

#[test]
fn build_async_overlaps_independent_async_transient_factories() {
    let state = async_transient_overlap_state();
    state.reset();

    let provider = block_on(ServiceProvider::<AsyncTransientOverlapRoot>::build_async())
        .expect("independent transient factories should be progressed together");
    assert_eq!(provider.root().left.label, "left transient");
    assert_eq!(provider.root().right.label, "right transient");

    let events = state
        .events
        .lock()
        .expect("async transient overlap mutex should not be poisoned")
        .clone();
    let first_finish = events
        .iter()
        .position(|event| event.ends_with(":finish"))
        .expect("both transient barrier futures should complete");

    assert_eq!(
        state.started.load(Ordering::SeqCst),
        2,
        "both independent transient factories must enter Pending"
    );
    assert_eq!(
        first_finish, 2,
        "neither transient factory may finish before both factories have started"
    );
    assert!(
        events[..first_finish]
            .iter()
            .all(|event| event.ends_with(":start")),
        "the overlap proof must be structural rather than timing based"
    );
}

#[test]
fn failed_async_build_drops_a_transient_child_after_its_committed_owner() {
    failure_events()
        .lock()
        .expect("failure rollback mutex should not be poisoned")
        .clear();

    let error = match block_on(ServiceProvider::<FailureRoot>::build_async()) {
        Ok(_) => panic!("the root factory intentionally fails"),
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("激活 provider"),
        "unexpected build error: {error}"
    );
    assert_eq!(
        *failure_events()
            .lock()
            .expect("failure rollback mutex should not be poisoned"),
        ["owner", "transient"],
        "failed activation must ordinary-drop a transient child only after its owning service"
    );
}

#[test]
fn cancelling_build_async_drops_the_in_flight_factory_before_owner_transient_rollback() {
    cancellation_events()
        .lock()
        .expect("cancellation rollback mutex should not be poisoned")
        .clear();
    CANCELLATION_ROOT_STARTED.store(false, Ordering::SeqCst);

    block_on(async {
        let mut build = Box::pin(ServiceProvider::<CancellationRoot>::build_async());

        for _ in 0..256 {
            std::future::poll_fn(|context| match build.as_mut().poll(context) {
                Poll::Ready(_) => {
                    panic!("the cancellation fixture's root factory must stay pending")
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
            "the root factory must be in flight before build_async is cancelled"
        );
        drop(build);

        for _ in 0..256 {
            if *cancellation_events()
                .lock()
                .expect("cancellation rollback mutex should not be poisoned")
                == ["root factory future", "owner", "transient"]
            {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("Tokio abort should release the factory frame before transient rollback");
    });

    assert_eq!(
        *cancellation_events()
            .lock()
            .expect("cancellation rollback mutex should not be poisoned"),
        ["root factory future", "owner", "transient"],
        "cancellation must discard the factory frame before ordinary-dropping its owner and child transient"
    );
}

#[test]
fn factory_parameter_transient_drops_after_its_completed_factory_future() {
    temporary_parameter_events()
        .lock()
        .expect("temporary parameter mutex should not be poisoned")
        .clear();

    let provider = block_on(ServiceProvider::<TemporaryParameterRoot>::build_async())
        .expect("a cleanup-free transient factory parameter should be activation-local");

    assert_eq!(
        *temporary_parameter_events()
            .lock()
            .expect("temporary parameter mutex should not be poisoned"),
        ["factory future", "transient"],
        "the completed factory future/frame must be gone before its temporary transient arena"
    );
    drop(provider);
}

#[test]
fn scope_shutdown_leaves_parent_singleton_transient_subtree_until_provider_shutdown() {
    let _lock = SCOPE_SHUTDOWN_TEST_LOCK
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    scope_shutdown_events()
        .lock()
        .expect("scope shutdown mutex should not be poisoned")
        .clear();

    let provider = block_on(ScopeProvider::<
        ScopeShutdownApp,
        ScopeLayer<ScopeShutdownRoot>,
    >::build_async())
    .expect("async scoped builder should accept cleanup on field transients");
    let scope = block_on(provider.create_scope_async())
        .expect("async scope creation should activate the scoped transient child");
    block_on(scope.shutdown());

    assert_eq!(
        *scope_shutdown_events()
            .lock()
            .expect("scope shutdown mutex should not be poisoned"),
        [
            "scope root:drop",
            "scope transient:cleanup",
            "scope transient:drop",
        ],
        "shutting down one scope must consume only its Scoped Arena and transient child subtree"
    );

    block_on(provider.shutdown());
    assert_eq!(
        *scope_shutdown_events()
            .lock()
            .expect("scope shutdown mutex should not be poisoned"),
        [
            "scope root:drop",
            "scope transient:cleanup",
            "scope transient:drop",
            "parent root:drop",
            "parent transient:cleanup",
            "parent transient:drop",
        ],
        "the parent Singleton transient must wait for explicit provider shutdown"
    );
}
