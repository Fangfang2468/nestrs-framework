//! 稳定实例的内部强 lease 与非递归同步释放。

use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use super::{ErasedService, ErasedServiceRef};
use crate::service::{Injectable, ServiceType};

/// 同一 provider 及其 scopes 共用的同步释放域；不依赖异步 executor。
#[derive(Default)]
pub(crate) struct ReleaseDomain {
    state: Mutex<ReleaseState>,
}

#[derive(Default)]
struct ReleaseState {
    pending: VecDeque<InstancePayload>,
    draining: bool,
}

impl ReleaseDomain {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn release(&self, payload: InstancePayload) {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.pending.push_back(payload);
            if state.draining {
                return;
            }
            state.draining = true;
        }

        let already_panicking = std::thread::panicking();
        let mut first_panic = None;
        loop {
            let payload = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                match state.pending.pop_front() {
                    Some(payload) => payload,
                    None => {
                        state.draining = false;
                        break;
                    }
                }
            };

            // No queue lock is held while arbitrary user destructors run. Reentrant record
            // destruction only appends another payload; it never recursively drops a service.
            if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(payload)))
                && first_panic.is_none()
            {
                first_panic = Some(panic);
            }
        }

        if let Some(panic) = first_panic
            && !already_panicking
        {
            resume_unwind(panic);
        }
    }
}

struct InstancePayload {
    // Drop the consumer before releasing its dependency edges, including when Drop unwinds.
    service: ErasedService,
    _dependencies: Vec<DependencyLease>,
}

struct InstanceRecord {
    payload: Option<InstancePayload>,
    domain: Arc<ReleaseDomain>,
}

impl Drop for InstanceRecord {
    fn drop(&mut self) {
        if let Some(payload) = self.payload.take() {
            self.domain.release(payload);
        }
    }
}

/// 保持服务地址及其依赖闭包存活的内部所有权凭证。
#[derive(Clone)]
pub(crate) struct DependencyLease(Arc<InstanceRecord>);

impl DependencyLease {
    pub(crate) fn new(
        service: ErasedService,
        dependencies: Vec<Self>,
        domain: Arc<ReleaseDomain>,
    ) -> Self {
        Self(Arc::new(InstanceRecord {
            payload: Some(InstancePayload {
                service,
                _dependencies: dependencies,
            }),
            domain,
        }))
    }

    fn service(&self) -> &ErasedService {
        &self
            .0
            .payload
            .as_ref()
            .expect("a leased instance cannot be in its destructor")
            .service
    }

    pub(crate) fn service_type(&self) -> ServiceType {
        self.service().service_type()
    }

    pub(crate) fn erased_ref(&self) -> ErasedServiceRef {
        ErasedServiceRef::new(self.clone())
    }

    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    pub(crate) fn pointer<T>(&self) -> Option<NonNull<T>>
    where
        T: Injectable + ?Sized,
    {
        self.service().pointer::<T>()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{DependencyLease, ReleaseDomain};
    use crate::activation::ErasedService;

    struct Counted(Arc<AtomicUsize>);

    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn deep_dependency_chain_releases_on_a_small_stack() {
        std::thread::Builder::new()
            .stack_size(128 * 1024)
            .spawn(|| {
                let drops = Arc::new(AtomicUsize::new(0));
                let domain = ReleaseDomain::new();
                let mut previous = None;
                for _ in 0..20_000 {
                    previous = Some(DependencyLease::new(
                        ErasedService::new(Counted(drops.clone())),
                        previous.into_iter().collect(),
                        domain.clone(),
                    ));
                }
                drop(previous);
                assert_eq!(drops.load(Ordering::SeqCst), 20_000);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn dependency_drop_waits_for_consumer_and_the_last_lease() {
        struct Ordered(&'static str, Arc<Mutex<Vec<&'static str>>>);
        impl Drop for Ordered {
            fn drop(&mut self) {
                self.1.lock().unwrap().push(self.0);
            }
        }
        let order = Arc::new(Mutex::new(Vec::new()));
        let domain = ReleaseDomain::new();
        let dependency = DependencyLease::new(
            ErasedService::new(Ordered("dependency", order.clone())),
            vec![],
            domain.clone(),
        );
        let consumer = DependencyLease::new(
            ErasedService::new(Ordered("consumer", order.clone())),
            vec![dependency.clone()],
            domain,
        );
        drop(consumer);
        assert_eq!(*order.lock().unwrap(), ["consumer"]);
        drop(dependency);
        assert_eq!(*order.lock().unwrap(), ["consumer", "dependency"]);
    }

    #[test]
    fn a_panicking_destructor_still_drains_dependencies_and_resets_the_domain() {
        struct Panics;
        impl Drop for Panics {
            fn drop(&mut self) {
                panic!("deliberate destructor panic");
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let domain = ReleaseDomain::new();
        let dependency = DependencyLease::new(
            ErasedService::new(Counted(drops.clone())),
            vec![],
            domain.clone(),
        );
        let consumer =
            DependencyLease::new(ErasedService::new(Panics), vec![dependency], domain.clone());
        assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(consumer))).is_err());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        drop(DependencyLease::new(
            ErasedService::new(Counted(drops.clone())),
            vec![],
            domain,
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 2);
    }
}
