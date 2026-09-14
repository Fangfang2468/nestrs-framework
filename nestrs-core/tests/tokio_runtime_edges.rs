//! Tokio-backed activation boundary regressions.
//!
//! These tests intentionally exercise only the public `ServiceProvider` API.  In particular,
//! they do not assume which Tokio worker runs a provider: a current-thread runtime and a
//! multi-thread runtime must both be valid activation hosts.

use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
    task::{Context, Poll, Waker},
};

use nestrs_core::{
    __private::{
        ActivationError, ClassProvider, ConstructionContext, ErasedService,
        FactoryConstructionContext, FactoryFuture, FactoryInvoker, FactoryProvider, Injectable,
        Lifetime, Provider, ProviderCommon, ServiceIdentifier, ServiceSource, ServiceType,
    },
    ServiceProvider,
};

struct PanickingWorkerRoot;

struct RuntimeContractRoot {
    construction: usize,
}

const PANICKING_PROVIDER_SOURCE_LINE: u32 = 101;
const RUNTIME_CONTRACT_PROVIDER_SOURCE_LINE: u32 = 102;

static RUNTIME_CONTRACT_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

fn identifier<T>() -> ServiceIdentifier
where
    T: Injectable + ?Sized,
{
    ServiceIdentifier::from(ServiceType::create::<T>())
}

fn source(line: u32) -> ServiceSource {
    ServiceSource::new("tests/tokio_runtime_edges.rs", line, 1)
}

fn singleton(line: u32) -> ProviderCommon {
    ProviderCommon {
        lifetime: Lifetime::Singleton,
        primary: false,
        source: source(line),
        cleanup: None,
    }
}

fn construct_panicking_worker_root(
    _context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    panic!("the Tokio activation worker must surface this panic as BuildError")
}

/// A one-yield factory future proves that both runtime kinds drive a spawned async worker,
/// without relying on wall-clock timing or a particular worker-thread identity.
struct YieldOnceFactoryFuture {
    yielded: bool,
}

impl Future for YieldOnceFactoryFuture {
    type Output = Result<ErasedService, ActivationError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.yielded {
            self.yielded = true;
            context.waker().wake_by_ref();
            return Poll::Pending;
        }

        let construction = RUNTIME_CONTRACT_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1;
        Poll::Ready(Ok(ErasedService::new(RuntimeContractRoot { construction })))
    }
}

fn construct_runtime_contract_root<'frame>(
    _context: FactoryConstructionContext<'frame>,
) -> FactoryFuture<'frame> {
    Box::pin(YieldOnceFactoryFuture { yielded: false })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn panicking_worker_root_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<PanickingWorkerRoot>(),
        common: singleton(PANICKING_PROVIDER_SOURCE_LINE),
        dependencies: Vec::new(),
        constructor: construct_panicking_worker_root,
    })
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn runtime_contract_root_provider() -> Provider {
    Provider::Factory(FactoryProvider {
        provide: identifier::<RuntimeContractRoot>(),
        common: singleton(RUNTIME_CONTRACT_PROVIDER_SOURCE_LINE),
        dependencies: Vec::new(),
        invoker: FactoryInvoker::Async(construct_runtime_contract_root),
    })
}

#[test]
fn build_async_requires_a_tokio_runtime_before_scheduling_workers() {
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    let mut build = Box::pin(ServiceProvider::<PanickingWorkerRoot>::build_async());

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
fn build_async_maps_a_panicking_class_worker_to_a_structured_error() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the current-thread Tokio runtime should build");

    let error = match runtime.block_on(ServiceProvider::<PanickingWorkerRoot>::build_async()) {
        Ok(_) => panic!("the intentionally panicking activation worker must fail the build"),
        Err(error) => error,
    };

    let message = error.to_string();
    assert!(
        message.contains("激活 worker")
            && message.contains("panic")
            && message.contains(&format!("{:?}", source(PANICKING_PROVIDER_SOURCE_LINE))),
        "unexpected build error: {message}"
    );
}

#[test]
fn build_async_accepts_current_and_multi_thread_tokio_runtimes_without_worker_affinity() {
    RUNTIME_CONTRACT_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let current_thread = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("the current-thread Tokio runtime should build");
    let current_provider = current_thread
        .block_on(ServiceProvider::<RuntimeContractRoot>::build_async())
        .expect("a current-thread Tokio runtime should safely drive async activation");
    assert_eq!(current_provider.root().construction, 1);
    drop(current_provider);
    drop(current_thread);

    let multi_thread = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .build()
        .expect("the multi-thread Tokio runtime should build");
    let multi_provider = multi_thread
        .block_on(ServiceProvider::<RuntimeContractRoot>::build_async())
        .expect("a multi-thread Tokio runtime should safely drive async activation");
    assert_eq!(multi_provider.root().construction, 2);

    assert_eq!(
        RUNTIME_CONTRACT_CONSTRUCTIONS.load(Ordering::SeqCst),
        2,
        "both runtime kinds must drive the async factory; worker allocation is intentionally unspecified"
    );
}
