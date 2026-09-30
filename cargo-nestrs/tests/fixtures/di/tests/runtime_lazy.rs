//! 延迟字段经过真实宏展开、自动绑定、静态图与协调器的黑盒回归。
//!
//! 同一测试二进制包含全部合法声明；每个用例建立独立容器。静态计数与门闩
//! 由测试锁隔离，避免并行测试把不同 root 的构造次数混在一起。
use nestrs::{factory, injectable};
use nestrs_core::{
    InitializationMode, LazyInjection, ServiceProvider, ServiceProviderOptions,
    get_required_service,
};
use std::{
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
static REPORT_BACKENDS: AtomicUsize = AtomicUsize::new(0);
static REPORTS: AtomicUsize = AtomicUsize::new(0);

struct ReportBackend;

#[factory]
async fn report_backend() -> ReportBackend {
    REPORT_BACKENDS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    ReportBackend
}

struct ReportService {
    rows: usize,
}

#[factory]
async fn reports(_backend: ReportBackend) -> ReportService {
    REPORTS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    ReportService { rows: 42 }
}

struct MissingReport;
trait MissingReportPort: Send + Sync {}

#[injectable]
struct OrderService {
    #[inject]
    #[lazy]
    reports: ReportService,
    #[inject]
    #[lazy]
    optional_report: Option<ReportService>,
    #[inject]
    #[lazy]
    absent: Option<MissingReport>,
    #[inject]
    #[lazy]
    absent_port: Option<dyn MissingReportPort>,
}

trait ReportPort: Send + Sync {
    fn name(&self) -> &'static str;
}

struct NamedReport(&'static str);

impl ReportPort for NamedReport {
    fn name(&self) -> &'static str {
        self.0
    }
}

#[factory(key = "sales")]
fn sales_report() -> NamedReport {
    NamedReport("sales")
}

#[factory(key = 7)]
fn inventory_report() -> NamedReport {
    NamedReport("inventory")
}

#[injectable]
struct KeyedReports {
    #[inject("sales")]
    #[lazy]
    sales: dyn ReportPort,
    // 辅助属性顺序及完整路径不能影响延迟标记的识别。
    #[nestrs::lazy]
    #[nestrs::inject(7)]
    inventory: dyn ReportPort,
    #[inject("sales")]
    #[lazy]
    sales_concrete: NamedReport,
    #[inject(7)]
    #[lazy]
    optional_inventory: Option<dyn ReportPort>,
    #[inject("7")]
    #[lazy]
    wrong_key_kind: Option<dyn ReportPort>,
}

struct Customer;

#[injectable]
struct ReportCache<T> {
    marker: PhantomData<T>,
    #[value(88)]
    capacity: usize,
}

#[injectable]
struct GenericReports<T> {
    #[inject]
    #[lazy]
    cache: ReportCache<T>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fields_defer_the_complete_target_closure_and_preserve_routes() {
    let _test = TEST_LOCK.lock().await;
    REPORT_BACKENDS.store(0, Ordering::SeqCst);
    REPORTS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    let orders = get_required_service!(provider, OrderService).await.unwrap();
    assert_eq!(REPORTS.load(Ordering::SeqCst), 0);
    assert_eq!(REPORT_BACKENDS.load(Ordering::SeqCst), 0);
    assert!(orders.absent.is_none() && orders.absent_port.is_none());

    let (report, optional) = tokio::join!(
        orders.reports.get(),
        orders.optional_report.as_ref().unwrap().get(),
    );
    let report = report.unwrap();
    assert_eq!(report.rows, 42);
    assert!(std::ptr::eq(report, optional.unwrap()));
    assert!(std::ptr::eq(report, orders.reports.get().await.unwrap()));
    assert_eq!(REPORTS.load(Ordering::SeqCst), 1);
    assert_eq!(REPORT_BACKENDS.load(Ordering::SeqCst), 1);

    let keyed = get_required_service!(provider, KeyedReports).await.unwrap();
    let sales = keyed.sales.get().await.unwrap();
    let concrete = keyed.sales_concrete.get().await.unwrap();
    assert_eq!(sales.name(), "sales");
    assert!(std::ptr::addr_eq(sales as *const dyn ReportPort, concrete));
    assert_eq!(keyed.inventory.get().await.unwrap().name(), "inventory");
    assert_eq!(
        keyed
            .optional_inventory
            .as_ref()
            .unwrap()
            .get()
            .await
            .unwrap()
            .name(),
        "inventory",
    );
    assert!(keyed.wrong_key_kind.is_none());

    let generic = get_required_service!(provider, GenericReports<Customer>)
        .await
        .unwrap();
    assert_eq!(generic.cache.get().await.unwrap().capacity, 88);
    provider.dispose_async().await.unwrap();
}

static SCOPED_TARGETS: AtomicUsize = AtomicUsize::new(0);
static TRANSIENT_TARGETS: AtomicUsize = AtomicUsize::new(0);

struct ScopedReport {
    id: usize,
}

#[factory(lifetime = Scoped)]
fn scoped_report() -> ScopedReport {
    ScopedReport {
        id: SCOPED_TARGETS.fetch_add(1, Ordering::SeqCst),
    }
}

struct TransientReport {
    id: usize,
}

#[factory(lifetime = Transient)]
fn transient_report() -> TransientReport {
    TransientReport {
        id: TRANSIENT_TARGETS.fetch_add(1, Ordering::SeqCst),
    }
}

#[injectable(lifetime = Scoped)]
struct ScopedConsumer {
    #[inject]
    #[lazy]
    shared: ReportService,
    #[inject]
    #[lazy]
    scoped: ScopedReport,
    #[inject]
    #[lazy]
    first: TransientReport,
    #[inject]
    #[lazy]
    second: TransientReport,
}

#[injectable(lifetime = Transient)]
struct DeferredScopeRequirement {
    #[inject]
    #[lazy]
    scoped: ScopedReport,
}

#[tokio::test]
async fn lazy_fields_preserve_scope_and_transient_occurrence_identity() {
    let _test = TEST_LOCK.lock().await;
    SCOPED_TARGETS.store(0, Ordering::SeqCst);
    TRANSIENT_TARGETS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    assert!(
        get_required_service!(provider, DeferredScopeRequirement)
            .await
            .is_err()
    );
    let first_scope = provider.create_scope();
    let second_scope = provider.create_scope();
    let first = get_required_service!(first_scope.service_provider(), ScopedConsumer)
        .await
        .unwrap();
    let second = get_required_service!(second_scope.service_provider(), ScopedConsumer)
        .await
        .unwrap();
    assert_eq!(SCOPED_TARGETS.load(Ordering::SeqCst), 0);
    assert_eq!(TRANSIENT_TARGETS.load(Ordering::SeqCst), 0);
    let (shared_first, shared_second) = tokio::join!(first.shared.get(), second.shared.get());
    assert!(std::ptr::eq(shared_first.unwrap(), shared_second.unwrap()));
    let first_scoped = first.scoped.get().await.unwrap();
    let second_scoped = second.scoped.get().await.unwrap();
    assert_ne!(first_scoped.id, second_scoped.id);
    let scoped_dependency =
        get_required_service!(first_scope.service_provider(), DeferredScopeRequirement)
            .await
            .unwrap();
    assert!(std::ptr::eq(
        first_scoped,
        scoped_dependency.scoped.get().await.unwrap(),
    ));
    let (left, right) = tokio::join!(first.first.get(), first.second.get());
    let left = left.unwrap();
    assert_ne!(left.id, right.unwrap().id);
    assert!(std::ptr::eq(left, first.first.get().await.unwrap()));
    assert_eq!(TRANSIENT_TARGETS.load(Ordering::SeqCst), 2);
    assert_eq!(SCOPED_TARGETS.load(Ordering::SeqCst), 2);
    first_scope.dispose_async().await.unwrap();
    second_scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}

static DELAY_STARTED: Semaphore = Semaphore::const_new(0);
static DELAY_RELEASE: Semaphore = Semaphore::const_new(0);
static DELAY_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

struct DelayedReport(usize);

#[factory(lifetime = Transient)]
async fn delayed_report() -> DelayedReport {
    let id = DELAY_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    DELAY_STARTED.add_permits(1);
    DELAY_RELEASE.acquire().await.unwrap().forget();
    DelayedReport(id)
}

#[injectable]
struct DelayedConsumer {
    #[inject]
    #[lazy]
    report: DelayedReport,
}

static FAILED_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static PANICKING_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
struct FailedReport;

#[factory(lifetime = Transient)]
async fn failed_report() -> Result<FailedReport, &'static str> {
    FAILED_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    Err("lazy-report-failure")
}

#[injectable(lifetime = Transient)]
struct FailedConsumer {
    #[inject]
    #[lazy]
    report: FailedReport,
}

struct PanickingReport;

#[factory(lifetime = Transient)]
async fn panicking_report() -> PanickingReport {
    PANICKING_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    panic!("lazy-report-panic");
}

#[injectable(lifetime = Transient)]
struct PanickingConsumer {
    #[inject]
    #[lazy]
    report: PanickingReport,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_waiters_do_not_duplicate_the_fixed_lazy_occurrence() {
    let _test = TEST_LOCK.lock().await;
    DELAY_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    FAILED_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    PANICKING_CONSTRUCTIONS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    let consumer = get_required_service!(provider, DelayedConsumer)
        .await
        .unwrap();
    let mut first = Box::pin(consumer.report.get());
    tokio::select! {
        permit = DELAY_STARTED.acquire() => permit.unwrap().forget(),
        _ = &mut first => panic!("延迟目标应等待测试释放"),
    }
    // 初始化已经被协调器接受。取消首个等待者后，其余等待者仍应取得同一实例。
    drop(first);
    DELAY_RELEASE.add_permits(1);
    let (second, third) = tokio::time::timeout(Duration::from_secs(5), async {
        tokio::join!(consumer.report.get(), consumer.report.get())
    })
    .await
    .expect("同一延迟槽位不能因取消而产生第二个阻塞 occurrence");
    let second = second.unwrap();
    assert_eq!(second.0, 0);
    assert!(std::ptr::eq(second, third.unwrap()));
    assert_eq!(DELAY_CONSTRUCTIONS.load(Ordering::SeqCst), 1);

    let failed = get_required_service!(provider, FailedConsumer)
        .await
        .unwrap();
    let (first_error, second_error) = tokio::join!(failed.report.get(), failed.report.get());
    let first_error = first_error.err().unwrap().to_string();
    assert!(first_error.contains("lazy-report-failure"));
    assert_eq!(first_error, second_error.err().unwrap().to_string());
    assert_eq!(FAILED_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert!(failed.report.get().await.is_err());
    assert_eq!(FAILED_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    // Transient 的另一消费槽位仍是一次独立尝试。
    let another = get_required_service!(provider, FailedConsumer)
        .await
        .unwrap();
    assert!(another.report.get().await.is_err());
    assert_eq!(FAILED_CONSTRUCTIONS.load(Ordering::SeqCst), 2);

    let panicking = get_required_service!(provider, PanickingConsumer)
        .await
        .unwrap();
    let error = panicking.report.get().await.err().unwrap().to_string();
    assert!(error.contains("lazy-report-panic"));
    assert_eq!(
        error,
        panicking.report.get().await.err().unwrap().to_string()
    );
    assert_eq!(PANICKING_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    provider.dispose_async().await.unwrap();
}

static CLEANUP_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static LATE_DROPS: AtomicUsize = AtomicUsize::new(0);
static ESCAPED: Mutex<Option<LazyInjection<LateReport>>> = Mutex::new(None);

struct LateReport {
    value: usize,
}

impl Drop for LateReport {
    fn drop(&mut self) {
        LATE_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

async fn cleanup_late_report() {
    CLEANUP_ORDER.lock().unwrap().push("report");
}

#[factory(lifetime = Transient, cleanup = "cleanup_late_report")]
fn late_report() -> LateReport {
    LateReport { value: 73 }
}

async fn cleanup_late_consumer() {
    CLEANUP_ORDER.lock().unwrap().push("consumer-start");
    tokio::task::yield_now().await;
    CLEANUP_ORDER.lock().unwrap().push("consumer-end");
}

#[injectable(lifetime = Transient, cleanup = "cleanup_late_consumer")]
struct LateConsumer {
    #[inject]
    #[lazy]
    report: Option<LateReport>,
}

impl Drop for LateConsumer {
    fn drop(&mut self) {
        *ESCAPED.lock().unwrap() = self.report.take();
    }
}

async fn cleanup_bad_consumer() {
    CLEANUP_ORDER.lock().unwrap().push("bad-consumer");
    panic!("lazy-cleanup-panic");
}

#[injectable(lifetime = Transient, cleanup = "cleanup_bad_consumer")]
struct BadCleanupConsumer {
    #[inject]
    #[lazy]
    report: LateReport,
}

#[tokio::test]
async fn late_dependencies_cleanup_after_consumers_and_escaped_leases_remain_safe() {
    let _test = TEST_LOCK.lock().await;
    CLEANUP_ORDER.lock().unwrap().clear();
    LATE_DROPS.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build().await.unwrap();
    let consumer = get_required_service!(provider, LateConsumer).await.unwrap();
    assert_eq!(
        consumer.report.as_ref().unwrap().get().await.unwrap().value,
        73
    );
    provider.dispose_async().await.unwrap();
    assert_eq!(
        *CLEANUP_ORDER.lock().unwrap(),
        ["consumer-start", "consumer-end", "report"],
    );
    assert_eq!(LATE_DROPS.load(Ordering::SeqCst), 0);
    let escaped = ESCAPED.lock().unwrap().take().unwrap();
    assert_eq!(escaped.get().await.unwrap().value, 73);
    drop(escaped);
    assert_eq!(LATE_DROPS.load(Ordering::SeqCst), 1);

    // 从未开始获取的句柄不能在 owner 已关闭后新建实例。
    let unused = ServiceProvider::build().await.unwrap();
    get_required_service!(unused, LateConsumer).await.unwrap();
    unused.dispose_async().await.unwrap();
    let escaped_unused = ESCAPED.lock().unwrap().take().unwrap();
    assert!(escaped_unused.get().await.is_err());
    assert_eq!(LATE_DROPS.load(Ordering::SeqCst), 1);

    CLEANUP_ORDER.lock().unwrap().clear();
    let panics = ServiceProvider::build().await.unwrap();
    get_required_service!(panics, BadCleanupConsumer)
        .await
        .unwrap()
        .report
        .get()
        .await
        .unwrap();
    let error = panics.dispose_async().await.unwrap_err();
    assert!(error.to_string().contains("lazy-cleanup-panic"));
    assert_eq!(error.failures().len(), 1);
    assert_eq!(*CLEANUP_ORDER.lock().unwrap(), ["bad-consumer", "report"]);
    assert_eq!(LATE_DROPS.load(Ordering::SeqCst), 2);
}

#[injectable]
struct ConstructionGuard {
    #[inject]
    #[lazy]
    target: TransientReport,
}

struct ConstructionProbe {
    diagnostic: String,
}

#[factory(lifetime = Transient)]
async fn construction_probe(guard: ConstructionGuard) -> ConstructionProbe {
    let diagnostic = match guard.target.get().await {
        Ok(_) => panic!("构造任务不能启动尚未初始化的延迟槽位"),
        Err(error) => error.to_string(),
    };
    ConstructionProbe { diagnostic }
}

#[tokio::test]
async fn unresolved_lazy_access_during_construction_fails_instead_of_deadlocking() {
    let _test = TEST_LOCK.lock().await;
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        max_concurrent_activations: NonZeroUsize::new(1).unwrap(),
        ..Default::default()
    })
    .await
    .unwrap();
    let probe = tokio::time::timeout(
        Duration::from_secs(5),
        get_required_service!(provider, ConstructionProbe),
    )
    .await
    .expect("单个构造名额下不能等待自己释放名额")
    .unwrap();
    assert!(probe.diagnostic.contains("构造"));
    // 阶段限制不应变成目标的永久失败；业务阶段仍能正常初始化它。
    let guard = get_required_service!(provider, ConstructionGuard)
        .await
        .unwrap();
    guard.target.get().await.unwrap();
    provider.dispose_async().await.unwrap();
}

static EAGER_TARGETS: AtomicUsize = AtomicUsize::new(0);
static EAGER_DEFERRED: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct EagerTarget {
    #[value(EAGER_TARGETS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
}

#[injectable(lifetime = Transient)]
struct EagerDeferred {
    #[value(EAGER_DEFERRED.fetch_add(1, Ordering::SeqCst))]
    id: usize,
}

#[injectable]
struct EagerConsumer {
    #[inject]
    #[lazy]
    target: EagerTarget,
    #[inject]
    #[lazy]
    deferred: EagerDeferred,
}

#[tokio::test]
async fn eager_selection_is_independent_of_a_consumers_lazy_field() {
    let _test = TEST_LOCK.lock().await;
    EAGER_TARGETS.store(0, Ordering::SeqCst);
    EAGER_DEFERRED.store(0, Ordering::SeqCst);
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(EAGER_TARGETS.load(Ordering::SeqCst), 1);
    assert_eq!(EAGER_DEFERRED.load(Ordering::SeqCst), 0);
    let consumer = get_required_service!(provider, EagerConsumer)
        .await
        .unwrap();
    assert_eq!(consumer.target.get().await.unwrap().id, 0);
    assert_eq!(consumer.deferred.get().await.unwrap().id, 0);
    assert_eq!(EAGER_TARGETS.load(Ordering::SeqCst), 1);
    assert_eq!(EAGER_DEFERRED.load(Ordering::SeqCst), 1);
    provider.dispose_async().await.unwrap();
}

static CLOSING_STARTED: Semaphore = Semaphore::const_new(0);
static CLOSING_RELEASE: Semaphore = Semaphore::const_new(0);
static CLOSING_DROPPED: Semaphore = Semaphore::const_new(0);
static CLOSING_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static CLOSING_DROPS: AtomicUsize = AtomicUsize::new(0);
static CLOSING_ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

struct ClosingReport;

impl Drop for ClosingReport {
    fn drop(&mut self) {
        CLOSING_DROPS.fetch_add(1, Ordering::SeqCst);
        CLOSING_DROPPED.add_permits(1);
    }
}

async fn cleanup_closing_report() {
    CLOSING_ORDER.lock().unwrap().push("report");
}

#[factory(lifetime = Transient, cleanup = "cleanup_closing_report")]
async fn closing_report() -> ClosingReport {
    CLOSING_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    CLOSING_STARTED.add_permits(1);
    CLOSING_RELEASE.acquire().await.unwrap().forget();
    ClosingReport
}

async fn cleanup_closing_consumer() {
    CLOSING_ORDER.lock().unwrap().push("consumer-start");
    tokio::task::yield_now().await;
    CLOSING_ORDER.lock().unwrap().push("consumer-end");
}

#[injectable(lifetime = Scoped, cleanup = "cleanup_closing_consumer")]
struct ClosingConsumer {
    #[inject]
    #[lazy]
    report: ClosingReport,
}

#[tokio::test]
async fn accepted_lazy_work_drains_after_both_query_and_disposal_are_cancelled() {
    let _test = TEST_LOCK.lock().await;
    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    // scope 预热会创建 Scoped 消费者，延迟的 Transient 仍保持未构造。
    scope.warm_up().await.unwrap();
    assert_eq!(CLOSING_CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    let consumer = get_required_service!(scope.service_provider(), ClosingConsumer)
        .await
        .unwrap();
    let mut query = Box::pin(consumer.report.get());
    tokio::select! {
        permit = CLOSING_STARTED.acquire() => permit.unwrap().forget(),
        _ = &mut query => panic!("延迟目标应等待测试释放"),
    }
    drop(query);

    let mut disposal = Box::pin(scope.dispose_async());
    tokio::select! {
        // 优先 poll disposal，确保 Close 已提交；目标还在等待，因此此处不能完成。
        biased;
        result = &mut disposal => panic!("已接受的初始化尚未结束：{result:?}"),
        _ = tokio::task::yield_now() => {},
    }
    drop(disposal);
    assert!(CLOSING_ORDER.lock().unwrap().is_empty());
    CLOSING_RELEASE.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), CLOSING_DROPPED.acquire())
        .await
        .expect("取消关闭等待不能取消实际关闭")
        .unwrap()
        .forget();
    assert_eq!(CLOSING_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(CLOSING_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(
        *CLOSING_ORDER.lock().unwrap(),
        ["consumer-start", "consumer-end", "report"],
    );
    provider.dispose_async().await.unwrap();
}
