use std::{
    future::Future,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll, Waker},
};

use super::{FactoryFuture, FactoryInputs};
use crate::activation::{
    ActivationPreparation, DependencyLease, ErasedService, InputSlot, ReleaseDomain,
    prepare_required,
};

struct Dependency(Arc<AtomicUsize>);
impl Drop for Dependency {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn pending_adapter(mut inputs: FactoryInputs<'_>) -> FactoryFuture<'_> {
    Box::pin(async move {
        let dependency = inputs.take::<Dependency>(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        std::future::pending::<()>().await;
        Ok(ErasedService::new(dependency.0.load(Ordering::SeqCst)))
    })
}

#[test]
fn an_owned_send_worker_keeps_factory_borrows_alive_until_cancellation() {
    let drops = Arc::new(AtomicUsize::new(0));
    let dependency = DependencyLease::new(
        ErasedService::new(Dependency(drops.clone())),
        vec![],
        ReleaseDomain::new(),
    );
    let mut preparation = ActivationPreparation::new(1);
    preparation
        .prepare(
            InputSlot::new(0),
            prepare_required::<Dependency>,
            Some(dependency),
        )
        .unwrap();
    let mut frame = preparation.finish_factory().unwrap();
    let mut worker = Box::pin(async move { pending_adapter(frame.inputs()).await });
    fn assert_send<T: Send>(_: &T) {}
    assert_send(&worker);
    let waker = Waker::noop();
    assert!(matches!(
        worker.as_mut().poll(&mut Context::from_waker(waker)),
        Poll::Pending
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(worker);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn successful_factory_frame_transfers_its_dependency_leases() {
    let drops = Arc::new(AtomicUsize::new(0));
    let dependency = DependencyLease::new(
        ErasedService::new(Dependency(drops.clone())),
        vec![],
        ReleaseDomain::new(),
    );
    let mut preparation = ActivationPreparation::new(1);
    preparation
        .prepare(
            InputSlot::new(0),
            prepare_required::<Dependency>,
            Some(dependency),
        )
        .unwrap();
    let mut frame = preparation.finish_factory().unwrap();
    {
        let mut inputs = frame.inputs();
        let _dependency = inputs.take::<Dependency>(InputSlot::new(0)).unwrap();
        inputs.ensure_all_consumed().unwrap();
    }
    let retained = frame.into_dependencies();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(retained);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
