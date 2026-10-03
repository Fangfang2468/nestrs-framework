//! 显式立即预热沿用原有构造失败、取消与清理协议；延迟策略不引入第二套执行器。
use nestrs::{factory, injectable, lazy};
use nestrs_core::{BuildError, ServiceProvider};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

static FAIL: AtomicBool = AtomicBool::new(true);
static BLOCK: AtomicBool = AtomicBool::new(false);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CLEANUP: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static STARTED: Semaphore = Semaphore::const_new(0);
static RELEASE: Semaphore = Semaphore::const_new(0);
static CLOSED: Semaphore = Semaphore::const_new(0);

async fn cleanup_base() {
    CLEANUP.lock().unwrap().push("base");
    CLOSED.add_permits(1);
}
#[lazy]
#[injectable(cleanup = "cleanup_base")]
struct Base;

struct ForcedClient;
async fn cleanup_client() {
    CLEANUP.lock().unwrap().push("client");
}
#[factory(cleanup = "cleanup_client")]
#[lazy(false)]
async fn forced_client(_base: Base) -> Result<ForcedClient, &'static str> {
    ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    if FAIL.load(Ordering::SeqCst) {
        return Err("startup-client-failed");
    }
    if BLOCK.load(Ordering::SeqCst) {
        STARTED.add_permits(1);
        RELEASE.acquire().await.unwrap().forget();
    }
    Ok(ForcedClient)
}

struct DeferredFailure;
static DEFERRED_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
#[lazy]
#[factory]
fn deferred_failure() -> Result<DeferredFailure, &'static str> {
    DEFERRED_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    Err("deferred-client-failed")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forced_startup_failure_and_cancelled_build_finish_cleanup_without_retrying() {
    let failure = ServiceProvider::build(None)
        .await
        .err()
        .expect("全局 Lazy 也必须等待 #[lazy(false)] 服务");
    assert!(matches!(failure, BuildError::Initialization { .. }));
    assert!(failure.to_string().contains("startup-client-failed"));
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
    assert_eq!(*CLEANUP.lock().unwrap(), ["base"]);
    assert_eq!(DEFERRED_ATTEMPTS.load(Ordering::SeqCst), 0);
    CLOSED.acquire().await.unwrap().forget();

    FAIL.store(false, Ordering::SeqCst);
    BLOCK.store(true, Ordering::SeqCst);
    CLEANUP.lock().unwrap().clear();
    let mut build = Box::pin(ServiceProvider::build(None));
    tokio::time::timeout(Duration::from_secs(5), async {
        tokio::select! {
            permit = STARTED.acquire() => permit.unwrap().forget(),
            _ = &mut build => panic!("build 必须等待尚未完成的强制预热工厂"),
        }
    })
    .await
    .unwrap();
    drop(build);
    RELEASE.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), CLOSED.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(*CLEANUP.lock().unwrap(), ["client", "base"]);
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);

    // 新 root 有独立实例状态。延迟 Singleton 的失败仍缓存到该 root 关闭。
    BLOCK.store(false, Ordering::SeqCst);
    let provider = ServiceProvider::build(None).await.unwrap();
    let first = provider
        .get_required_service::<DeferredFailure>()
        .await
        .err()
        .unwrap()
        .to_string();
    let second = provider
        .get_required_service::<DeferredFailure>()
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(first.contains("deferred-client-failed"));
    assert_eq!(first, second);
    assert_eq!(DEFERRED_ATTEMPTS.load(Ordering::SeqCst), 1);
    provider.dispose_async().await.unwrap();
}
