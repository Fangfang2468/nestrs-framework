//! 已选定依赖到固定构造输入的事务式准备。

use super::{
    ConstructionError, ConstructionInputs, FactoryLeaseFrame, InputPreparer, InputSlot,
    inputs::InputBuffer,
};
use crate::activation::DependencyLease;

pub(crate) struct ActivationPreparation {
    buffer: InputBuffer,
    dependencies: Vec<DependencyLease>,
}

impl ActivationPreparation {
    pub(crate) fn new(slot_count: usize) -> Self {
        Self {
            buffer: InputBuffer::new(slot_count),
            dependencies: Vec::with_capacity(slot_count),
        }
    }

    pub(crate) fn prepare(
        &mut self,
        slot: InputSlot,
        preparer: InputPreparer,
        input: Option<DependencyLease>,
    ) -> Result<(), ConstructionError> {
        let prepared = preparer(slot, input.as_ref().map(DependencyLease::erased_ref))?;
        // Retain the actual returned token owner, not merely the supplied input: safe handwritten
        // preparers can return a previously prepared value backed by another instance.
        let dependency = prepared.dependency();
        self.buffer.insert(slot, prepared)?;
        if let Some(dependency) = dependency {
            self.dependencies.push(dependency);
        }
        Ok(())
    }

    pub(crate) fn finish_class(
        self,
    ) -> Result<(ConstructionInputs, Vec<DependencyLease>), ConstructionError> {
        Ok((self.buffer.finish()?, self.dependencies))
    }

    pub(crate) fn finish_factory(self) -> Result<FactoryLeaseFrame, ConstructionError> {
        let (inputs, dependencies) = self.finish_class()?;
        Ok(FactoryLeaseFrame::new(inputs, dependencies))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::ActivationPreparation;
    use crate::activation::{
        ConstructionError, DependencyLease, ErasedService, InputSlot, ReleaseDomain,
        prepare_required,
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
}
