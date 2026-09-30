use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::ActivationPreparation;
use crate::activation::{
    ConstructionError, DependencyLease, ErasedService, InputSlot, ReleaseDomain, prepare_required,
};

struct Counted(Arc<AtomicUsize>);
impl Drop for Counted {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn a_failed_slot_write_does_not_retain_the_rejected_dependency() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut preparation = ActivationPreparation::new(1);
    let first = DependencyLease::new(ErasedService::new(3_u32), vec![], ReleaseDomain::new());
    preparation
        .prepare(InputSlot::new(0), prepare_required::<u32>, Some(first))
        .unwrap();
    let rejected = DependencyLease::new(
        ErasedService::new(Counted(drops.clone())),
        vec![],
        ReleaseDomain::new(),
    );
    assert!(matches!(
        preparation.prepare(
            InputSlot::new(0),
            prepare_required::<Counted>,
            Some(rejected)
        ),
        Err(ConstructionError::SlotAlreadyPrepared { .. })
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let (mut inputs, dependencies) = preparation.finish_class().unwrap();
    assert_eq!(dependencies.len(), 1);
    assert_eq!(*inputs.take::<u32>(InputSlot::new(0)).unwrap(), 3);
}

#[test]
fn failed_preparation_does_not_fill_a_slot() {
    let mut preparation = ActivationPreparation::new(1);
    assert!(matches!(
        preparation.prepare(InputSlot::new(0), prepare_required::<u32>, None),
        Err(ConstructionError::RequiredDependencyAbsent { .. })
    ));
    assert!(matches!(
        preparation.finish_class(),
        Err(ConstructionError::UnfilledSlot { .. })
    ));
}
