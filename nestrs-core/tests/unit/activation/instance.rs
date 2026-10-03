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
    let parent = DependencyLease::new(ErasedService::new(1_u32), vec![dependency], domain.clone());
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
        // 后初始化释放上下文，确保 TLS 销毁时先销毁它，再销毁 HELD。
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

mod panic_payload {
    use super::{Counted, DependencyLease, ErasedService, ReleaseDomain};
    use std::{
        future::Future,
        panic::{AssertUnwindSafe, catch_unwind, panic_any},
        pin::pin,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        task::{Context, Poll, Waker},
        time::Duration,
    };

    const WAIT: Duration = Duration::from_secs(5);

    fn without_unwind<T>(operation: impl FnOnce() -> T) -> T {
        match catch_unwind(AssertUnwindSafe(operation)) {
            Ok(value) => value,
            Err(payload) => {
                // 测试失败本身不能再次析构恶意 payload，导致整套测试双重 panic abort。
                std::mem::forget(payload);
                panic!("panic payload 析构逃逸了释放边界");
            }
        }
    }

    fn ready<F: Future>(future: F) -> F::Output {
        let mut future = pin!(future);
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("drainer 已结束，但释放回执仍未完成"),
        }
    }

    struct Blocking {
        entered: mpsc::Sender<()>,
        resume: Mutex<mpsc::Receiver<()>>,
    }

    impl Drop for Blocking {
        fn drop(&mut self) {
            self.entered.send(()).unwrap();
            self.resume.get_mut().unwrap().recv_timeout(WAIT).unwrap();
        }
    }

    struct BlockedDrainer {
        resume: Option<mpsc::Sender<()>>,
        finished: mpsc::Receiver<bool>,
    }

    impl BlockedDrainer {
        fn start(domain: &Arc<ReleaseDomain>) -> Self {
            let (entered, wait_entered) = mpsc::channel();
            let (resume, wait_resume) = mpsc::channel();
            let (finished, wait_finished) = mpsc::channel();
            let lease = DependencyLease::new(
                ErasedService::new(Blocking {
                    entered,
                    resume: Mutex::new(wait_resume),
                }),
                vec![],
                domain.clone(),
            );
            let control = Self {
                resume: Some(resume),
                finished: wait_finished,
            };
            std::thread::spawn(move || {
                let succeeded = match catch_unwind(AssertUnwindSafe(|| drop(lease))) {
                    Ok(()) => true,
                    Err(payload) => {
                        std::mem::forget(payload);
                        false
                    }
                };
                let _ = finished.send(succeeded);
            });
            wait_entered.recv_timeout(WAIT).unwrap();
            control
        }

        fn finish(mut self) -> bool {
            self.resume.take().unwrap().send(()).unwrap();
            self.finished.recv_timeout(WAIT).unwrap()
        }
    }

    impl Drop for BlockedDrainer {
        fn drop(&mut self) {
            // 即使主测试先断言失败，辅助线程也不被遗留在等待中。
            if let Some(resume) = self.resume.take() {
                let _ = resume.send(());
            }
        }
    }

    struct ThrowsPayload<T: Send + 'static>(Mutex<Option<T>>);

    impl<T: Send + 'static> Drop for ThrowsPayload<T> {
        fn drop(&mut self) {
            let payload = self.0.get_mut().unwrap().take().unwrap();
            panic_any(payload);
        }
    }

    struct Payload {
        drops: Arc<AtomicUsize>,
        dependency: Option<DependencyLease>,
    }

    impl Drop for Payload {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
            drop(self.dependency.take());
            panic!("panic payload destructor sentinel");
        }
    }

    fn throwing_lease(
        domain: &Arc<ReleaseDomain>,
        drops: &Arc<AtomicUsize>,
        dependency: Option<DependencyLease>,
    ) -> DependencyLease {
        DependencyLease::new(
            ErasedService::new(ThrowsPayload(Mutex::new(Some(Payload {
                drops: drops.clone(),
                dependency,
            })))),
            vec![],
            domain.clone(),
        )
    }

    fn counted(domain: &Arc<ReleaseDomain>, drops: &Arc<AtomicUsize>) -> DependencyLease {
        DependencyLease::new(
            ErasedService::new(Counted(drops.clone())),
            vec![],
            domain.clone(),
        )
    }

    #[test]
    fn queued_ordinary_panic_payload_drop_does_not_stop_the_drainer() {
        let domain = ReleaseDomain::new();
        let drainer = BlockedDrainer::start(&domain);
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let drops = Arc::new(AtomicUsize::new(0));
        // 当前线程是独立提交者；它的无回执组不能把 payload panic 抛给另一个 drainer。
        drop(throwing_lease(&domain, &payload_drops, None));
        drop(counted(&domain, &drops));
        let completion = counted(&domain, &drops).release_tracked().unwrap();
        assert!(drainer.finish(), "另一个线程的 payload Drop 杀死了 drainer");
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(without_unwind(|| ready(completion)).is_empty());
        assert!(ready(counted(&domain, &drops).release_tracked().unwrap()).is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn tracked_panic_payload_drop_cannot_escape_completion() {
        let domain = ReleaseDomain::new();
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let completion = without_unwind(|| {
            throwing_lease(&domain, &payload_drops, None)
                .release_tracked()
                .unwrap()
        });
        let errors = without_unwind(|| ready(completion));
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|error| error.contains("ThrowsPayload")));
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn abandoning_completed_tracked_panic_payload_is_safe() {
        let domain = ReleaseDomain::new();
        let payload_drops = Arc::new(AtomicUsize::new(0));
        without_unwind(|| {
            let completion = throwing_lease(&domain, &payload_drops, None)
                .release_tracked()
                .unwrap();
            drop(completion);
        });
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
        let drops = Arc::new(AtomicUsize::new(0));
        assert!(ready(counted(&domain, &drops).release_tracked().unwrap()).is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn abandoning_queued_tracked_panic_payload_keeps_draining() {
        let domain = ReleaseDomain::new();
        let drainer = BlockedDrainer::start(&domain);
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let completion = throwing_lease(&domain, &payload_drops, None)
            .release_tracked()
            .unwrap();
        drop(completion);
        let drops = Arc::new(AtomicUsize::new(0));
        drop(counted(&domain, &drops));
        assert!(drainer.finish(), "已放弃回执的 payload Drop 杀死了 drainer");
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn tracked_panic_payload_reentrant_leases_keep_their_original_completion_group() {
        struct Panics(&'static str, Arc<AtomicUsize>);
        impl Drop for Panics {
            fn drop(&mut self) {
                self.1.fetch_add(1, Ordering::SeqCst);
                panic!("{}", self.0);
            }
        }
        let domain = ReleaseDomain::new();
        let drainer = BlockedDrainer::start(&domain);
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let dependency_drops = Arc::new(AtomicUsize::new(0));
        let dependency = DependencyLease::new(
            ErasedService::new(Panics(
                "reentrant dependency sentinel",
                dependency_drops.clone(),
            )),
            vec![],
            domain.clone(),
        );
        let completion = throwing_lease(&domain, &payload_drops, Some(dependency))
            .release_tracked()
            .unwrap();
        let unrelated_drops = Arc::new(AtomicUsize::new(0));
        let unrelated = DependencyLease::new(
            ErasedService::new(Panics(
                "unrelated release sentinel",
                unrelated_drops.clone(),
            )),
            vec![],
            domain.clone(),
        )
        .release_tracked()
        .unwrap();
        assert!(drainer.finish());
        let errors = without_unwind(|| ready(completion));
        assert!(
            errors
                .iter()
                .any(|error| error.contains("reentrant dependency sentinel"))
        );
        assert!(
            errors
                .iter()
                .all(|error| !error.contains("unrelated release sentinel"))
        );
        let unrelated_errors = without_unwind(|| ready(unrelated));
        assert_eq!(unrelated_errors.len(), 1);
        assert!(unrelated_errors[0].contains("unrelated release sentinel"));
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
        assert_eq!(dependency_drops.load(Ordering::SeqCst), 1);
        assert_eq!(unrelated_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn ordinary_untracked_drop_preserves_the_original_panic_payload() {
        struct Original(Arc<AtomicUsize>);
        impl Drop for Original {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        let domain = ReleaseDomain::new();
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let lease = DependencyLease::new(
            ErasedService::new(ThrowsPayload(Mutex::new(Some(Original(
                payload_drops.clone(),
            ))))),
            vec![],
            domain.clone(),
        );
        let payload = catch_unwind(AssertUnwindSafe(|| drop(lease))).unwrap_err();
        assert!(payload.is::<Original>());
        assert_eq!(payload_drops.load(Ordering::SeqCst), 0);
        drop(payload);
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
        let drops = Arc::new(AtomicUsize::new(0));
        drop(counted(&domain, &drops));
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    struct CrossDomainOriginal(Arc<AtomicUsize>);

    impl Drop for CrossDomainOriginal {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct ReleasesNestedDomain {
        dependency: Option<DependencyLease>,
        continued: Arc<AtomicUsize>,
    }

    impl Drop for ReleasesNestedDomain {
        fn drop(&mut self) {
            drop(self.dependency.take());
            // 重入释放不能在外层 service 的 Drop 中途恢复依赖 panic。
            self.continued.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn cross_domain_lease(
        payload_drops: &Arc<AtomicUsize>,
        continued: &Arc<AtomicUsize>,
    ) -> DependencyLease {
        let dependency = DependencyLease::new(
            ErasedService::new(ThrowsPayload(Mutex::new(Some(CrossDomainOriginal(
                payload_drops.clone(),
            ))))),
            vec![],
            ReleaseDomain::new(),
        );
        DependencyLease::new(
            ErasedService::new(ReleasesNestedDomain {
                dependency: Some(dependency),
                continued: continued.clone(),
            }),
            vec![],
            ReleaseDomain::new(),
        )
    }

    #[test]
    fn cross_domain_ordinary_release_preserves_original_payload_after_outer_drop_finishes() {
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let continued = Arc::new(AtomicUsize::new(0));
        let lease = cross_domain_lease(&payload_drops, &continued);
        let result = catch_unwind(AssertUnwindSafe(|| drop(lease)));
        assert_eq!(continued.load(Ordering::SeqCst), 1);
        let payload = result.expect_err("跨域依赖的原始 panic 被吞掉");
        assert!(payload.is::<CrossDomainOriginal>());
        assert_eq!(payload_drops.load(Ordering::SeqCst), 0);
        drop(payload);
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cross_domain_tracked_release_collects_nested_failure_without_unwinding_outer_drop() {
        let payload_drops = Arc::new(AtomicUsize::new(0));
        let continued = Arc::new(AtomicUsize::new(0));
        let lease = cross_domain_lease(&payload_drops, &continued);
        let errors = without_unwind(|| ready(lease.release_tracked().unwrap()));
        assert_eq!(continued.load(Ordering::SeqCst), 1);
        assert_eq!(payload_drops.load(Ordering::SeqCst), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("ThrowsPayload"));
        assert!(errors[0].contains("CrossDomainOriginal"));
    }
}
