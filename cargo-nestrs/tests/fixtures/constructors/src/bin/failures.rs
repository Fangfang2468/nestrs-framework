//! Result 构造失败属于运行期；共享失败缓存、依赖保活与 cleanup 保持原有契约。
use nestrs::{constructor, injectable};
use nestrs_core::ServiceProvider;
use std::sync::atomic::{AtomicUsize, Ordering};

static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static TRANSIENT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CLEANUPS: AtomicUsize = AtomicUsize::new(0);

async fn cleanup() {
    CLEANUPS.fetch_add(1, Ordering::SeqCst);
}
#[injectable(cleanup = "cleanup")]
struct Foundation;
impl Foundation {
    #[constructor]
    fn create() -> Self {
        Self
    }
}

#[derive(Debug)]
struct StartupFailure(&'static str);
#[injectable]
struct Broken;
impl Broken {
    #[constructor]
    fn create(_foundation: Foundation) -> Result<Self, StartupFailure> {
        ATTEMPTS.fetch_add(1, Ordering::SeqCst);
        let failure = StartupFailure("constructor-rejected");
        assert_eq!(failure.0, "constructor-rejected");
        Err(failure)
    }
}
#[injectable(lifetime = Transient)]
struct Retry;
impl Retry {
    #[constructor]
    fn create() -> Result<Self, &'static str> {
        TRANSIENT_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
        Err("transient-constructor-rejected")
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 0);
    let first = provider
        .get_required_service::<Broken>()
        .await
        .err()
        .unwrap()
        .to_string();
    let second = provider
        .get_required_service::<Broken>()
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(first, second);
    assert!(first.contains("constructor-rejected"));
    assert!(first.contains("Broken"));
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
    assert_eq!(CLEANUPS.load(Ordering::SeqCst), 0);
    // 失败的消费者不应误清理仍可供其他请求使用的成功依赖。
    provider.get_required_service::<Foundation>().await.unwrap();
    for _ in 0..2 {
        let error = provider
            .get_required_service::<Retry>()
            .await
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("transient-constructor-rejected"));
    }
    assert_eq!(TRANSIENT_ATTEMPTS.load(Ordering::SeqCst), 2);
    provider.dispose_async().await.unwrap();
    assert_eq!(CLEANUPS.load(Ordering::SeqCst), 1);
    println!("constructor failure contracts passed");
}
