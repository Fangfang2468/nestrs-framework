//! Factory provider 构造 adapter 的 ABI。

use std::{future::Future, pin::Pin};

use super::{ConstructionError, ConstructionInputs, InputSlot};
use crate::{
    activation::{DependencyLease, erased_service::ErasedService},
    service::Injectable,
};

/// 异步 factory adapter 返回的 frame-bound future。
pub type FactoryFuture<'frame> =
    Pin<Box<dyn Future<Output = Result<ErasedService, ConstructionError>> + Send + 'frame>>;

/// 同步 factory adapter 的单态化签名。
pub type FactoryConstructor =
    for<'frame> fn(FactoryInputs<'frame>) -> Result<ErasedService, ConstructionError>;

/// 异步 factory adapter 的单态化签名。
pub type AsyncConstructor = for<'frame> fn(FactoryInputs<'frame>) -> FactoryFuture<'frame>;

/// 仅供 factory adapter 消费的 frame-bound 构造输入。
///
/// 只有持有全部真实 dependency leases 的 `FactoryLeaseFrame` 可以创建它。
#[doc(hidden)]
pub struct FactoryInputs<'frame> {
    inputs: ConstructionInputs,
    _lease_frame: &'frame [DependencyLease],
}

/// 可被 owned worker 持有、但只能借出一次 inputs 的 factory 调用帧。
pub(crate) struct FactoryLeaseFrame {
    inputs: Option<ConstructionInputs>,
    dependencies: Vec<DependencyLease>,
}

impl FactoryLeaseFrame {
    pub(super) fn new(inputs: ConstructionInputs, dependencies: Vec<DependencyLease>) -> Self {
        Self {
            inputs: Some(inputs),
            dependencies,
        }
    }

    pub(crate) fn inputs(&mut self) -> FactoryInputs<'_> {
        FactoryInputs {
            inputs: self
                .inputs
                .take()
                .expect("factory frame inputs may only be consumed once"),
            _lease_frame: &self.dependencies,
        }
    }

    pub(crate) fn into_dependencies(self) -> Vec<DependencyLease> {
        self.dependencies
    }
}

impl<'frame> FactoryInputs<'frame> {
    /// 取走一个只在本次 factory 调用期间有效的必选服务借用。
    pub fn take<T>(&mut self, slot: InputSlot) -> Result<&'frame T, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.inputs.take::<T>(slot)?;

        // SAFETY: FactoryInputs can only be constructed by FactoryLeaseFrame, which
        // keeps every dependency lease alive for 'frame.
        Ok(unsafe { token.into_ptr().as_ref() })
    }

    /// 取走一个只在本次 factory 调用期间有效的可选服务借用。
    pub fn take_optional<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<&'frame T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.inputs.take_optional::<T>(slot)?;

        Ok(token.map(|token| {
            // SAFETY: see `Self::take`; absent inputs contain no address.
            unsafe { token.into_ptr().as_ref() }
        }))
    }

    /// 拒绝 factory adapter 未消费的 descriptor 槽位。
    pub fn ensure_all_consumed(&self) -> Result<(), ConstructionError> {
        self.inputs.ensure_all_consumed()
    }
}

#[cfg(test)]
mod tests {
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
}
