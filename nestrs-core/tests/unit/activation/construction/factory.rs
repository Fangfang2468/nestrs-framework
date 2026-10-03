use crate::activation::construction::FactoryLeaseFrame;

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
    ConstructionInput, ConstructionInputs, DependencyLease, ErasedService, InputKind, InputSlot,
    ReleaseDomain, project_required,
};
use crate::service::ServiceType;

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
    let inputs = ConstructionInputs::new(vec![ConstructionInput::immediate(
        ServiceType::create::<Dependency>(),
        InputKind::Required,
        dependency,
        project_required::<Dependency>,
    )])
    .unwrap();
    let mut frame = FactoryLeaseFrame::new(inputs);
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
    let inputs = ConstructionInputs::new(vec![ConstructionInput::immediate(
        ServiceType::create::<Dependency>(),
        InputKind::Required,
        dependency,
        project_required::<Dependency>,
    )])
    .unwrap();
    let mut frame = FactoryLeaseFrame::new(inputs);
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

#[test]
fn a_factory_cannot_borrow_a_same_type_substitute_outside_its_frame() {
    use crate::activation::ConstructionError;
    let drops = Arc::new(AtomicUsize::new(0));
    let dependency = DependencyLease::new(
        ErasedService::new(Dependency(drops.clone())),
        vec![],
        ReleaseDomain::new(),
    );
    let inputs = ConstructionInputs::new(vec![ConstructionInput::immediate(
        ServiceType::create::<Dependency>(),
        InputKind::Required,
        dependency,
        |slot, input, target| {
            let (pointer, _owner) = input.cast::<Dependency>().unwrap();
            // SAFETY: the retained owner keeps this exact checked Dependency alive.
            let drops = unsafe { pointer.as_ref() }.0.clone();
            let substitute = DependencyLease::new(
                ErasedService::new(Dependency(drops)),
                vec![],
                ReleaseDomain::new(),
            );
            project_required::<Dependency>(slot, substitute.erased_ref(), target)
        },
    )])
    .unwrap();
    let mut frame = FactoryLeaseFrame::new(inputs);
    let mut inputs = frame.inputs();
    for expected_drops in [1, 2] {
        assert!(matches!(
            inputs.take::<Dependency>(InputSlot::new(0)),
            Err(ConstructionError::ProjectionOwnerMismatch { .. })
        ));
        assert_eq!(drops.load(Ordering::SeqCst), expected_drops);
    }
    drop(inputs);
    assert_eq!(
        drops.load(Ordering::SeqCst),
        2,
        "the frame still owns the original instance"
    );
    drop(frame);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}
