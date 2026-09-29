//! 稳定实例的内部强 lease 与非递归同步释放。

use std::{
    any::Any,
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    pin::Pin,
    ptr::NonNull,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
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
    pending: VecDeque<PendingRelease>,
    draining: bool,
}

impl ReleaseDomain {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn release(&self, payload: InstancePayload) {
        let active = ACTIVE_RELEASE
            .try_with(|active| active.borrow().clone())
            .ok()
            .flatten();
        let nested = active.is_some();
        let group = active.unwrap_or_default();
        self.enqueue(payload, group.clone());
        // Reentrant dependency drops contribute to their initiating release. A queued
        // release from another thread has its own result and must not poison this caller.
        if !nested
            && !std::thread::panicking()
            && let Some(failures) = group.take_completed()
            && let Some(failure) = failures.into_iter().next()
        {
            resume_unwind(failure.panic);
        }
    }

    fn release_tracked(&self, payload: InstancePayload) -> ReleaseCompletion {
        let group = Arc::new(ReleaseGroup::default());
        self.enqueue(payload, group.clone());
        ReleaseCompletion(group)
    }

    fn enqueue(&self, payload: InstancePayload, group: Arc<ReleaseGroup>) {
        group.begin();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.pending.push_back(PendingRelease { payload, group });
            if state.draining {
                return;
            }
            state.draining = true;
        }
        loop {
            let pending = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                match state.pending.pop_front() {
                    Some(pending) => pending,
                    None => {
                        state.draining = false;
                        break;
                    }
                }
            };
            let PendingRelease { payload, group } = pending;
            let service = payload.service.service_type().name;
            // No queue lock is held during user destructors. Any dependency payloads
            // they enqueue join this release group, preserving iterative destruction.
            let context = ReleaseContext::enter(group.clone());
            let panic = catch_unwind(AssertUnwindSafe(|| drop(payload))).err();
            drop(context);
            group.finish(panic.map(|panic| ReleaseFailure { service, panic }));
        }
    }
}

struct PendingRelease {
    payload: InstancePayload,
    group: Arc<ReleaseGroup>,
}

thread_local! {
    static ACTIVE_RELEASE: RefCell<Option<Arc<ReleaseGroup>>> = const { RefCell::new(None) };
}

struct ReleaseContext(Option<Arc<ReleaseGroup>>);
impl ReleaseContext {
    fn enter(group: Arc<ReleaseGroup>) -> Self {
        Self(
            ACTIVE_RELEASE
                .try_with(|active| active.replace(Some(group)))
                .ok()
                .flatten(),
        )
    }
}
impl Drop for ReleaseContext {
    fn drop(&mut self) {
        let _ = ACTIVE_RELEASE.try_with(|active| active.replace(self.0.take()));
    }
}

struct ReleaseFailure {
    service: &'static str,
    panic: Box<dyn Any + Send>,
}

#[derive(Default)]
struct ReleaseProgress {
    pending: usize,
    failures: Vec<ReleaseFailure>,
    waker: Option<Waker>,
}

#[derive(Default)]
struct ReleaseGroup(Mutex<ReleaseProgress>);
impl ReleaseGroup {
    fn begin(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending += 1;
    }

    fn finish(&self, failure: Option<ReleaseFailure>) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.failures.extend(failure);
            state.pending -= 1;
            if state.pending == 0 {
                state.waker.take()
            } else {
                None
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    fn take_completed(&self) -> Option<Vec<ReleaseFailure>> {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        (state.pending == 0).then(|| std::mem::take(&mut state.failures))
    }
}

/// An executor-independent receipt for the payload and its reentrant dependency releases.
/// Dropping the receipt does not cancel destruction. It never retains an instance lease.
pub(crate) struct ReleaseCompletion(Arc<ReleaseGroup>);
impl Future for ReleaseCompletion {
    type Output = Vec<String>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let failures = {
            let mut state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
            if state.pending != 0 {
                state.waker = Some(context.waker().clone());
                return Poll::Pending;
            }
            std::mem::take(&mut state.failures)
        };
        Poll::Ready(
            failures
                .into_iter()
                .map(|failure| {
                    let detail = failure
                        .panic
                        .downcast_ref::<String>()
                        .map(String::as_str)
                        .or_else(|| failure.panic.downcast_ref::<&str>().copied())
                        .unwrap_or("未提供字符串 panic 信息");
                    format!("{}: {detail}", failure.service)
                })
                .collect(),
        )
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

    /// Only a last lease starts tracked destruction. An escaped lease keeps the
    /// allocation alive but must not make logical owner shutdown wait for its holder.
    pub(crate) fn release_tracked(self) -> Option<ReleaseCompletion> {
        Arc::into_inner(self.0).map(|mut record| {
            let payload = record
                .payload
                .take()
                .expect("a last lease owns its payload");
            record.domain.release_tracked(payload)
        })
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

    #[tokio::test]
    async fn tracked_release_observes_reentrant_dependency_panics_and_does_not_wait_for_escape() {
        struct Panics;
        impl Drop for Panics {
            fn drop(&mut self) {
                panic!("dependency destructor sentinel");
            }
        }
        let domain = ReleaseDomain::new();
        let dependency = DependencyLease::new(ErasedService::new(Panics), vec![], domain.clone());
        let parent =
            DependencyLease::new(ErasedService::new(1_u32), vec![dependency], domain.clone());
        let escaped = parent.clone();
        assert!(parent.release_tracked().is_none());
        let errors = escaped.release_tracked().unwrap().await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Panics: dependency destructor sentinel"));
        let last = DependencyLease::new(ErasedService::new(1_u32), vec![], domain);
        assert!(last.release_tracked().unwrap().await.is_empty());
    }

    #[test]
    fn escaped_leases_release_during_thread_local_teardown() {
        thread_local! {
            static HELD: std::cell::RefCell<Option<DependencyLease>> = const { std::cell::RefCell::new(None) };
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let observed = drops.clone();
        std::thread::spawn(move || {
            let domain = ReleaseDomain::new();
            HELD.with(|held| {
                *held.borrow_mut() = Some(DependencyLease::new(
                    ErasedService::new(Counted(observed)),
                    vec![],
                    domain.clone(),
                ))
            });
            // Initialize the release context later so TLS destroys it before HELD.
            drop(DependencyLease::new(
                ErasedService::new(0_u32),
                vec![],
                domain,
            ));
        })
        .join()
        .unwrap();
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
