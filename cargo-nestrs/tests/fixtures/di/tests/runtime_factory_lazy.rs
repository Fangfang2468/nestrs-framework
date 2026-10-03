//! factory 的延迟参数必须成为可移入服务的 owned token，不能借用临时 factory frame。
//! 这里通过真实工具链覆盖签名改写、冻结依赖选择、owner 归属及关闭协议。
use nestrs::{factory, injectable};
use nestrs_core::{LazyInjection, ServiceProvider, ServiceProviderOptions};
use std::{
    future::Future,
    marker::PhantomData,
    num::NonZeroUsize,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Semaphore;

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TARGETS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct Configuration {
    #[value(42)]
    value: usize,
}

struct Report(usize);
#[factory]
async fn report() -> Report {
    TARGETS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    Report(73)
}

struct SyncConsumer {
    report: LazyInjection<Report>,
    configured: usize,
}
#[factory]
fn sync_consumer(configuration: Configuration, #[lazy] report: Report) -> SyncConsumer {
    SyncConsumer {
        report,
        configured: configuration.value,
    }
}

struct AsyncConsumer {
    report: LazyInjection<Report>,
    configured: usize,
}
#[factory]
async fn async_consumer(#[lazy] report: Report, configuration: Configuration) -> AsyncConsumer {
    let before = configuration.value;
    tokio::task::yield_now().await;
    assert_eq!(before, configuration.value);
    AsyncConsumer {
        report,
        configured: configuration.value,
    }
}

struct FutureConsumer {
    report: LazyInjection<Report>,
    configured: usize,
}
#[factory]
fn future_consumer(
    configuration: Configuration,
    #[inject]
    #[lazy]
    report: Report,
) -> impl Future<Output = Result<FutureConsumer, &'static str>> {
    async move {
        tokio::task::yield_now().await;
        Ok(FutureConsumer {
            report,
            configured: configuration.value,
        })
    }
}

trait Port: Send + Sync {
    fn name(&self) -> &'static str;
}
struct KeyedReport;
impl Port for KeyedReport {
    fn name(&self) -> &'static str {
        "sales"
    }
}
#[factory(key = "sales")]
fn keyed_report() -> KeyedReport {
    KeyedReport
}
struct Missing;
struct Customer;
#[injectable]
struct Cache<T> {
    marker: PhantomData<T>,
}
struct Routes {
    sales: LazyInjection<dyn Port>,
    optional: Option<LazyInjection<dyn Port>>,
    absent: Option<LazyInjection<Missing>>,
    wrong_key: Option<LazyInjection<dyn Port>>,
    generic: LazyInjection<Cache<Customer>>,
}
#[factory]
fn routes(
    #[inject("sales")]
    #[lazy]
    sales: dyn Port,
    #[nestrs::lazy]
    #[nestrs::inject("sales")]
    optional: Option<dyn Port>,
    #[lazy] absent: Option<Missing>,
    #[inject(7)]
    #[lazy]
    wrong_key: Option<dyn Port>,
    #[lazy] generic: Cache<Customer>,
) -> Routes {
    Routes {
        sales,
        optional,
        absent,
        wrong_key,
        generic,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_async_and_explicit_future_factories_move_lazy_tokens_out_of_frames() {
    let _test = TEST_LOCK.lock().await;
    TARGETS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    let sync = provider
        .get_required_service::<SyncConsumer>()
        .await
        .unwrap();
    let asynchronous = provider
        .get_required_service::<AsyncConsumer>()
        .await
        .unwrap();
    let future = provider
        .get_required_service::<FutureConsumer>()
        .await
        .unwrap();
    assert_eq!(
        (sync.configured, asynchronous.configured, future.configured),
        (42, 42, 42)
    );
    assert_eq!(
        TARGETS.load(Ordering::SeqCst),
        0,
        "交付 token 不应构造延迟目标"
    );
    let (first, second, third) = tokio::join!(
        sync.report.get(),
        asynchronous.report.get(),
        future.report.get()
    );
    let first = first.unwrap();
    assert_eq!(first.0, 73);
    assert!(std::ptr::eq(first, second.unwrap()) && std::ptr::eq(first, third.unwrap()));
    assert_eq!(TARGETS.load(Ordering::SeqCst), 1);
    let routes = provider.get_required_service::<Routes>().await.unwrap();
    assert!(routes.absent.is_none() && routes.wrong_key.is_none());
    let sales = routes.sales.get().await.unwrap();
    assert_eq!(sales.name(), "sales");
    assert!(std::ptr::eq(
        sales,
        routes.optional.as_ref().unwrap().get().await.unwrap()
    ));
    routes.generic.get().await.unwrap();
    provider.dispose_async().await.unwrap();
}

static SCOPED: AtomicUsize = AtomicUsize::new(0);
static TRANSIENT: AtomicUsize = AtomicUsize::new(0);
struct Session(usize);
#[factory(lifetime = Scoped)]
fn session() -> Session {
    Session(SCOPED.fetch_add(1, Ordering::SeqCst))
}
struct Occurrence(usize);
#[factory(lifetime = Transient)]
fn occurrence() -> Occurrence {
    Occurrence(TRANSIENT.fetch_add(1, Ordering::SeqCst))
}
struct ScopedConsumer {
    session: LazyInjection<Session>,
    first: LazyInjection<Occurrence>,
    second: LazyInjection<Occurrence>,
}
#[factory(lifetime = Scoped)]
fn scoped_consumer(
    #[lazy] session: Session,
    #[lazy] first: Occurrence,
    #[lazy] second: Occurrence,
) -> ScopedConsumer {
    ScopedConsumer {
        session,
        first,
        second,
    }
}
struct RequiresScope {
    session: LazyInjection<Session>,
}
#[factory(lifetime = Transient)]
fn requires_scope(#[lazy] session: Session) -> RequiresScope {
    RequiresScope { session }
}

#[tokio::test]
async fn delayed_factory_parameters_keep_scope_and_per_slot_transient_identity() {
    let _test = TEST_LOCK.lock().await;
    SCOPED.store(0, Ordering::SeqCst);
    TRANSIENT.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    assert!(
        provider
            .get_required_service::<RequiresScope>()
            .await
            .is_err()
    );
    let left_scope = provider.create_scope();
    let right_scope = provider.create_scope();
    let left = left_scope
        .service_provider()
        .get_required_service::<ScopedConsumer>()
        .await
        .unwrap();
    let right = right_scope
        .service_provider()
        .get_required_service::<ScopedConsumer>()
        .await
        .unwrap();
    assert_eq!(SCOPED.load(Ordering::SeqCst), 0);
    assert_eq!(TRANSIENT.load(Ordering::SeqCst), 0);
    let left_session = left.session.get().await.unwrap();
    assert_ne!(left_session.0, right.session.get().await.unwrap().0);
    let other = left_scope
        .service_provider()
        .get_required_service::<RequiresScope>()
        .await
        .unwrap();
    assert!(std::ptr::eq(
        left_session,
        other.session.get().await.unwrap()
    ));
    let (first, second, same) = tokio::join!(left.first.get(), left.second.get(), left.first.get());
    let first = first.unwrap();
    assert!(std::ptr::eq(first, same.unwrap()));
    assert_ne!(first.0, second.unwrap().0);
    assert_eq!(TRANSIENT.load(Ordering::SeqCst), 2);
    left_scope.dispose_async().await.unwrap();
    right_scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}

struct ConstructionProbe {
    target: LazyInjection<Occurrence>,
    error: String,
}
#[factory]
async fn construction_probe(#[lazy] target: Occurrence) -> ConstructionProbe {
    let error = target
        .get()
        .await
        .err()
        .expect("构造阶段不能启动未就绪 lazy 目标")
        .to_string();
    ConstructionProbe { target, error }
}
#[tokio::test]
async fn construction_phase_guard_does_not_poison_the_owned_lazy_parameter() {
    let _test = TEST_LOCK.lock().await;
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        max_concurrent_activations: NonZeroUsize::new(1).unwrap(),
        ..Default::default()
    })
    .await
    .unwrap();
    let probe = tokio::time::timeout(
        Duration::from_secs(5),
        provider.get_required_service::<ConstructionProbe>(),
    )
    .await
    .expect("构造 worker 不能等待自己释放唯一名额")
    .unwrap();
    assert!(probe.error.contains("构造"));
    probe.target.get().await.unwrap();
    provider.dispose_async().await.unwrap();
}

static FAILED: AtomicUsize = AtomicUsize::new(0);
struct Broken;
#[factory(lifetime = Transient)]
async fn broken() -> Result<Broken, &'static str> {
    FAILED.fetch_add(1, Ordering::SeqCst);
    Err("factory-lazy-target-failed")
}
struct BrokenConsumer {
    target: LazyInjection<Broken>,
}
#[factory(lifetime = Transient)]
fn broken_consumer(#[lazy] target: Broken) -> BrokenConsumer {
    BrokenConsumer { target }
}
#[tokio::test]
async fn lazy_parameter_failure_is_cached_per_token_not_per_transient_provider() {
    let _test = TEST_LOCK.lock().await;
    FAILED.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    let consumer = provider
        .get_required_service::<BrokenConsumer>()
        .await
        .unwrap();
    assert_eq!(FAILED.load(Ordering::SeqCst), 0);
    let (first, second) = tokio::join!(consumer.target.get(), consumer.target.get());
    let error = first.err().unwrap().to_string();
    assert!(error.contains("factory-lazy-target-failed"));
    assert_eq!(error, second.err().unwrap().to_string());
    assert_eq!(FAILED.load(Ordering::SeqCst), 1);
    let another = provider
        .get_required_service::<BrokenConsumer>()
        .await
        .unwrap();
    assert!(another.target.get().await.is_err());
    assert_eq!(FAILED.load(Ordering::SeqCst), 2);
    provider.dispose_async().await.unwrap();
}

static STARTED: Semaphore = Semaphore::const_new(0);
static RELEASE: Semaphore = Semaphore::const_new(0);
static TARGET_CLEANED: Semaphore = Semaphore::const_new(0);
static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static ESCAPED: Mutex<Option<LazyInjection<Delayed>>> = Mutex::new(None);
static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Delayed;
impl Drop for Delayed {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}
async fn cleanup_delayed() {
    ORDER.lock().unwrap().push("target");
    TARGET_CLEANED.add_permits(1);
}
#[factory(lifetime = Transient, cleanup = "cleanup_delayed")]
async fn delayed() -> Delayed {
    STARTED.add_permits(1);
    RELEASE.acquire().await.unwrap().forget();
    Delayed
}
struct ClosingConsumer {
    target: Option<LazyInjection<Delayed>>,
}
impl Drop for ClosingConsumer {
    fn drop(&mut self) {
        *ESCAPED.lock().unwrap() = self.target.take();
    }
}
async fn cleanup_consumer() {
    ORDER.lock().unwrap().push("consumer-start");
    tokio::task::yield_now().await;
    ORDER.lock().unwrap().push("consumer-end");
}
#[factory(lifetime = Scoped, cleanup = "cleanup_consumer")]
fn closing_consumer(#[lazy] target: Option<Delayed>) -> ClosingConsumer {
    ClosingConsumer { target }
}

#[tokio::test]
async fn cancelled_lazy_wait_and_disposal_drain_work_and_keep_escaped_tokens_safe() {
    let _test = TEST_LOCK.lock().await;
    ORDER.lock().unwrap().clear();
    DROPS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    let consumer = scope
        .service_provider()
        .get_required_service::<ClosingConsumer>()
        .await
        .unwrap();
    let mut query = Box::pin(consumer.target.as_ref().unwrap().get());
    tokio::select! {
        permit = STARTED.acquire() => permit.unwrap().forget(),
        _ = &mut query => panic!("延迟目标应等待测试释放"),
    }
    drop(query);
    let mut disposal = Box::pin(scope.dispose_async());
    tokio::select! {
        biased;
        result = &mut disposal => panic!("仍需排空已接受任务：{result:?}"),
        _ = tokio::task::yield_now() => {},
    }
    drop(disposal);
    RELEASE.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), TARGET_CLEANED.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    assert_eq!(
        *ORDER.lock().unwrap(),
        ["consumer-start", "consumer-end", "target"]
    );
    let escaped = ESCAPED
        .lock()
        .unwrap()
        .take()
        .expect("返回服务拥有 token，可以在 Drop 时移出");
    escaped.get().await.unwrap();
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    // 等 root 确认 scope 关闭，避免把 journal 最后一次释放与下面的断言竞争。
    provider.dispose_async().await.unwrap();
    drop(escaped);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);

    let unused = ServiceProvider::build().await.unwrap();
    let scope = unused.create_scope();
    scope
        .service_provider()
        .get_required_service::<ClosingConsumer>()
        .await
        .unwrap();
    scope.dispose_async().await.unwrap();
    let unused_token = ESCAPED.lock().unwrap().take().unwrap();
    assert!(
        unused_token.get().await.is_err(),
        "owner 关闭后不能开始新构造"
    );
    unused.dispose_async().await.unwrap();
}
