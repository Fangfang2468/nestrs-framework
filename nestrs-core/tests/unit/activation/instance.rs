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
