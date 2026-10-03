//! 真实 driver 生成的 Scoped factory 在创建失败、清理失败与取消时的 owner 契约。
use nestrs::{factory, injectable};
use nestrs_core::{InitializationMode, ServiceProvider, ServiceScopeOptions};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static MODE: AtomicUsize = AtomicUsize::new(0);
static CLEANUP_PANICS: AtomicBool = AtomicBool::new(false);
static CREATED: AtomicUsize = AtomicUsize::new(0);
static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static STARTED: Semaphore = Semaphore::const_new(0);
static RELEASE: Semaphore = Semaphore::const_new(0);
static DROPPED: Semaphore = Semaphore::const_new(0);

#[injectable]
struct RootService;

struct Dependency(usize);

impl Drop for Dependency {
    fn drop(&mut self) {
        EVENTS.lock().unwrap().push("dependency-drop");
        DROPPED.add_permits(1);
    }
}

async fn cleanup_dependency() {
    tokio::task::yield_now().await;
    EVENTS.lock().unwrap().push("dependency-cleanup");
    assert!(
        !CLEANUP_PANICS.load(Ordering::SeqCst),
        "scope cleanup failure sentinel"
    );
}

#[factory(lifetime = Scoped, cleanup = "cleanup_dependency")]
fn dependency() -> Dependency {
    Dependency(CREATED.fetch_add(1, Ordering::SeqCst))
}

struct Consumer;

impl Drop for Consumer {
    fn drop(&mut self) {
        EVENTS.lock().unwrap().push("consumer-drop");
    }
}

async fn cleanup_consumer() {
    tokio::task::yield_now().await;
    EVENTS.lock().unwrap().push("consumer-cleanup");
}

#[factory(lifetime = Scoped, cleanup = "cleanup_consumer")]
async fn consumer(dependency: Dependency) -> Result<Consumer, &'static str> {
    assert!(dependency.0 < 10);
    match MODE.load(Ordering::SeqCst) {
        0 => Err("scope creation failure sentinel"),
        1 => {
            STARTED.add_permits(1);
            RELEASE.acquire().await.unwrap().forget();
            Ok(Consumer)
        }
        _ => Ok(Consumer),
    }
}

fn reset(mode: usize, cleanup_panics: bool) {
    MODE.store(mode, Ordering::SeqCst);
    CLEANUP_PANICS.store(cleanup_panics, Ordering::SeqCst);
    CREATED.store(0, Ordering::SeqCst);
    EVENTS.lock().unwrap().clear();
    // 前一用例的正常关闭同样会发送完成信号，不能把它当成本次取消后的关闭。
    while let Ok(permit) = DROPPED.try_acquire() {
        permit.forget();
    }
}

#[tokio::test]
async fn failed_scope_creation_finishes_cleanup_and_keeps_root_available() {
    let _test = TEST_LOCK.lock().await;
    for cleanup_panics in [false, true] {
        reset(0, cleanup_panics);
        let provider = ServiceProvider::build(None).await.unwrap();
        let error = match provider
            .create_scope(Some(ServiceScopeOptions {
                initialization: InitializationMode::Eager,
            }))
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("失败的 Scoped factory 不得交付部分初始化的 scope"),
        };
        assert!(
            error
                .error
                .to_string()
                .contains("scope creation failure sentinel")
        );
        assert_eq!(error.dispose_error.is_some(), cleanup_panics);
        if let Some(dispose_error) = error.dispose_error {
            assert_eq!(dispose_error.failures().len(), 1);
            assert!(
                dispose_error
                    .to_string()
                    .contains("scope cleanup failure sentinel")
            );
        }
        assert_eq!(
            *EVENTS.lock().unwrap(),
            ["dependency-cleanup", "dependency-drop"],
            "错误交付前必须完成已创建实例的 cleanup 与释放"
        );
        provider
            .get_required_service::<RootService>()
            .await
            .unwrap();

        // 失败只属于未交付的 scope；新 scope 创建新实例，root 仍然接受查询。
        CLEANUP_PANICS.store(false, Ordering::SeqCst);
        MODE.store(2, Ordering::SeqCst);
        let next = provider.create_scope(None).await.unwrap();
        let dependency = next
            .service_provider()
            .get_required_service::<Dependency>()
            .await
            .unwrap();
        assert_eq!(dependency.0, 1);
        next.dispose_async().await.unwrap();
        // root 汇总其所有 scope 的历史 cleanup 失败，即使创建调用者已收到该错误。
        let disposal = provider.dispose_async().await;
        assert_eq!(disposal.is_err(), cleanup_panics);
        if let Err(error) = disposal {
            assert_eq!(error.failures().len(), 1);
            assert!(error.to_string().contains("scope cleanup failure sentinel"));
        }
    }
}

#[tokio::test]
async fn cancelled_scope_creation_drains_accepted_factory_before_cleanup() {
    let _test = TEST_LOCK.lock().await;
    reset(1, false);
    let provider = ServiceProvider::build(None).await.unwrap();
    let mut creating = Box::pin(provider.create_scope(Some(ServiceScopeOptions {
        initialization: InitializationMode::Eager,
    })));
    tokio::select! {
        permit = STARTED.acquire() => permit.unwrap().forget(),
        result = &mut creating => panic!("初始化应等待测试释放，result.is_ok()={}", result.is_ok()),
    }
    drop(creating);
    assert!(EVENTS.lock().unwrap().is_empty());
    RELEASE.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), DROPPED.acquire())
        .await
        .expect("取消创建等待后仍应完成初始化并关闭未交付的 scope")
        .unwrap()
        .forget();
    assert_eq!(
        *EVENTS.lock().unwrap(),
        [
            "consumer-cleanup",
            "consumer-drop",
            "dependency-cleanup",
            "dependency-drop"
        ]
    );
    provider
        .get_required_service::<RootService>()
        .await
        .unwrap();
    provider.dispose_async().await.unwrap();
}
