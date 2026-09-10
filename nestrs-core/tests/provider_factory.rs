use std::{
    future::Future,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

use nestrs_core::{
    __private::{
        ActivationError, ActivationFuture, ConstructionContext, ErasedService, FactoryInvoker,
    },
    registration::{service_source::ServiceSource, service_type::ServiceType},
};

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

fn block_on<F>(future: F) -> F::Output
where
    F: Future,
{
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);

    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

fn sync_factory(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(String::from("sync")))
}

fn async_factory(_context: ConstructionContext) -> ActivationFuture {
    Box::pin(async { Ok(ErasedService::new(String::from("async"))) })
}

fn failing_factory(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Err(ActivationError::FactoryFailed {
        provider: "failing_factory",
        provider_source: ServiceSource::new("provider_factory.rs", 1, 1),
    })
}

#[test]
fn factory_invoker_executes_sync_and_async_adapters_through_one_future_abi() {
    let sync = block_on(FactoryInvoker::Sync(sync_factory).invoke(ConstructionContext::new()))
        .expect("sync factory should resolve");
    assert_eq!(sync.service_type(), ServiceType::create::<String>());

    let asynchronous =
        block_on(FactoryInvoker::Async(async_factory).invoke(ConstructionContext::new()))
            .expect("async factory should resolve");
    assert_eq!(asynchronous.service_type(), ServiceType::create::<String>());
}

#[test]
fn factory_invoker_preserves_factory_failure_identity() {
    let result = block_on(FactoryInvoker::Sync(failing_factory).invoke(ConstructionContext::new()));

    assert!(matches!(
        result,
        Err(ActivationError::FactoryFailed {
            provider: "failing_factory",
            provider_source: ServiceSource { .. },
        })
    ));
}
