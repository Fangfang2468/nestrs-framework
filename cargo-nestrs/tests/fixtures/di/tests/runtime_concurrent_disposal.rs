use nestrs::injectable;
use nestrs_core::ServiceProvider;
use std::{
    sync::{
        Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
static STARTED: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(0);
static FAST_CLEANUP_STARTED: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(0);
static FAST_DROPS: AtomicUsize = AtomicUsize::new(0);
static DEP_CLEANED: AtomicBool = AtomicBool::new(false);
static DROP_SAW_CLOSED_DEP: AtomicBool = AtomicBool::new(false);
static RELEASE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
#[injectable(lifetime=Scoped)]
struct Slow;
impl Drop for Slow {
    fn drop(&mut self) {
        STARTED.add_permits(1);
        let guard = RELEASE.0.lock().unwrap();
        let _ = RELEASE
            .1
            .wait_timeout_while(guard, Duration::from_secs(5), |released| !*released)
            .unwrap();
    }
}
async fn close_dependency() {
    DEP_CLEANED.store(true, Ordering::SeqCst);
}
#[injectable(lifetime=Scoped,cleanup="close_dependency")]
struct Dependency;
impl Dependency {
    fn is_closed(&self) -> bool {
        DEP_CLEANED.load(Ordering::SeqCst)
    }
}
async fn fast_cleanup() {
    FAST_CLEANUP_STARTED.add_permits(1);
}
#[injectable(lifetime=Scoped,cleanup="fast_cleanup")]
struct Fast {
    #[inject]
    dependency: Dependency,
}
impl Drop for Fast {
    fn drop(&mut self) {
        FAST_DROPS.fetch_add(1, Ordering::SeqCst);
        DROP_SAW_CLOSED_DEP.store(self.dependency.is_closed(), Ordering::SeqCst);
        panic!("Fast Drop panic sentinel");
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_scope_disposal_waits_for_its_own_destructors_and_preserves_dependency_order() {
    let root = ServiceProvider::build(None).await.unwrap();
    let first = root.create_scope(None).await.unwrap();
    let second = root.create_scope(None).await.unwrap();
    first
        .service_provider()
        .get_required_service::<Slow>()
        .await
        .unwrap();
    second
        .service_provider()
        .get_required_service::<Fast>()
        .await
        .unwrap();
    let mut first_close = Box::pin(first.dispose_async());
    tokio::select! {
      value=&mut first_close=>panic!("first scope closed prematurely: {value:?}"),
      permit=STARTED.acquire()=>{permit.unwrap().forget();}
    }
    let mut second_close = Box::pin(second.dispose_async());
    tokio::select! {
      value=&mut second_close=>panic!("second scope closed before cleanup began: {value:?}"),
      permit=FAST_CLEANUP_STARTED.acquire()=>{permit.unwrap().forget();}
    }
    let premature = tokio::time::timeout(Duration::from_millis(50), &mut second_close).await;
    let drops_before_release = FAST_DROPS.load(Ordering::SeqCst);
    let dependency_closed_before_release = DEP_CLEANED.load(Ordering::SeqCst);
    *RELEASE.0.lock().unwrap() = true;
    RELEASE.1.notify_one();
    let first_result = first_close.await;
    let second_result = second_close.await;
    assert!(
        premature.is_err(),
        "scope must wait for its queued destructor"
    );
    assert_eq!(drops_before_release, 0);
    assert!(
        !dependency_closed_before_release,
        "dependency cleanup must wait for consumer destruction"
    );
    first_result.expect("another scope's destructor panic must not be attributed to this scope");
    let error = second_result.unwrap_err();
    assert_eq!(error.failures().len(), 1);
    assert!(error.to_string().contains("Fast Drop panic sentinel"));
    assert!(!error.to_string().contains("Slow"));
    assert_eq!(FAST_DROPS.load(Ordering::SeqCst), 1);
    assert!(DEP_CLEANED.load(Ordering::SeqCst));
    assert!(!DROP_SAW_CLOSED_DEP.load(Ordering::SeqCst));
    let root_error = root.dispose_async().await.unwrap_err();
    assert_eq!(root_error.failures(), error.failures());
}
