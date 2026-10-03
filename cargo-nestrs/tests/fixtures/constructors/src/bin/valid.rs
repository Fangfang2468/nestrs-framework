//! 原始业务字段经 constructor 参数关联后交付真实注入令牌；普通表达式仍是业务值。
#![deny(unused_imports)]
use constructor_library::{Clock, GenericWrapper, Outer, OuterPort, Repository, RepositoryPort};
use nestrs::{constructor, factory, injectable};
use nestrs_core::{Injection, LazyInjection, ServiceProvider};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

static ASYNC_READY: AtomicBool = AtomicBool::new(false);
static REPORTS: AtomicUsize = AtomicUsize::new(0);
static SESSIONS: AtomicUsize = AtomicUsize::new(0);
static TICKETS: AtomicUsize = AtomicUsize::new(0);

struct Database;
#[factory]
async fn database() -> Database {
    tokio::task::yield_now().await;
    ASYNC_READY.store(true, Ordering::SeqCst);
    Database
}

trait MailPort: Send + Sync {
    fn name(&self) -> &'static str;
}
#[injectable(key = "outbound")]
struct Mailer {
    name: &'static str,
}
impl Mailer {
    #[constructor]
    fn create() -> Self {
        Self { name: "mail" }
    }
}
impl MailPort for Mailer {
    fn name(&self) -> &'static str {
        self.name
    }
}

#[injectable(key = 7)]
struct Codec {
    value: usize,
}
impl Codec {
    #[constructor]
    fn create() -> Self {
        Self { value: 700 }
    }
}

struct Missing;
#[injectable]
struct Reports {
    id: usize,
}
impl Reports {
    #[constructor]
    fn create() -> Self {
        Self {
            id: REPORTS.fetch_add(1, Ordering::SeqCst),
        }
    }
}

#[injectable]
struct Checkout {
    database: Database,
    clock: Clock,
    mailer: dyn MailPort,
    codec: Codec,
    absent: Option<Missing>,
    reports: Reports,
    label: String,
}
impl Checkout {
    #[constructor]
    fn start(
        database_service: Database,
        clock_service: Clock,
        #[inject("outbound")] outbound: dyn MailPort,
        #[inject(7)] codec: Codec,
        absent: Option<Missing>,
        #[lazy] reports: Reports,
    ) -> Result<Self, &'static str> {
        assert!(
            ASYNC_READY.load(Ordering::SeqCst),
            "同步 constructor 执行前必须准备完异步依赖"
        );
        // 改名字段与连续简单别名都要对应原始依赖槽位，不产生第二份服务实例。
        let database_alias = database_service;
        let database = database_alias;
        let clock = clock_service;
        // 业务计算先取得普通值；这些值不应因来源于依赖的方法而变成注入令牌。
        let outbound_name = outbound.name();
        let clock_sequence = clock.sequence();
        let codec_value = codec.value;
        let label = format!("{outbound_name}:{clock_sequence}:{codec_value}");
        Ok(Self {
            database,
            clock,
            mailer: outbound,
            codec,
            absent,
            reports,
            label,
        })
    }
}

#[injectable]
struct ValidatedLabel {
    label: String,
}

#[injectable]
struct RawService {
    r#type: Clock,
}
impl RawService {
    // 元数据里的 Rust raw identifier 与 rustc 的实际标识符身份必须一致。
    #[constructor]
    fn r#type(r#type: Clock) -> Self {
        Self { r#type }
    }
}
impl ValidatedLabel {
    #[constructor]
    fn start(clock: Clock) -> Self {
        // 参数仅参与校验/生成普通值也仍是 graph 的依赖，不强迫把 token 放到字段中。
        let sequence = clock.sequence();
        assert_eq!(sequence, 0);
        Self {
            label: format!("clock:{sequence}"),
        }
    }
}

#[injectable]
struct LazyOptions {
    mailer: dyn MailPort,
    reports: Option<Reports>,
    absent: Option<Missing>,
}
impl LazyOptions {
    #[constructor]
    fn create(
        #[inject("outbound")]
        #[lazy]
        mailer: dyn MailPort,
        #[lazy] reports: Option<Reports>,
        #[lazy] absent: Option<Missing>,
    ) -> Self {
        Self {
            mailer,
            reports,
            absent,
        }
    }
}

#[injectable(lifetime = Scoped)]
struct Session {
    id: usize,
}
impl Session {
    #[constructor]
    fn create() -> Self {
        Self {
            id: SESSIONS.fetch_add(1, Ordering::SeqCst),
        }
    }
}
#[injectable(lifetime = Transient)]
struct Ticket {
    id: usize,
}
impl Ticket {
    #[constructor]
    fn create() -> Self {
        Self {
            id: TICKETS.fetch_add(1, Ordering::SeqCst),
        }
    }
}
#[injectable(lifetime = Scoped)]
struct RequestService {
    session: Session,
    first: Ticket,
    second: Ticket,
}
impl RequestService {
    #[constructor]
    fn create(session: Session, first: Ticket, second: Ticket) -> Self {
        Self {
            session,
            first,
            second,
        }
    }
}

#[injectable]
struct Configured {
    value: usize,
}
impl Configured {
    #[cfg(not(feature = "alternate"))]
    #[constructor]
    fn standard() -> Self {
        Self { value: 71 }
    }
    #[cfg(feature = "alternate")]
    #[constructor]
    fn alternate() -> Self {
        Self { value: 91 }
    }
    #[cfg(any())]
    #[constructor]
    async fn excluded<T>(&self) -> Self {
        panic!("cfg 排除后不可见")
    }
}

macro_rules! generated_service {
    () => {
        #[injectable]
        struct Generated {
            clock: Clock,
            value: usize,
        }
        impl Generated {
            #[constructor]
            fn create(source: Clock) -> Self {
                Self {
                    clock: source,
                    value: 33,
                }
            }
        }
    };
}
generated_service!();

#[injectable]
struct ExternalService {
    clock: Clock,
    value: usize,
}
#[path = "../external_impl.rs"]
mod external_impl;

struct Customer;
struct Order;

struct OrdinaryBusinessService;
impl OrdinaryBusinessService {
    // 同名普通业务定义没有工具生成的卫生身份，不能被内部 ABI 审计误杀。
    const __NESTRS_CONSTRUCTOR: usize = 17;
    fn __nestrs_constructor_dependencies() -> usize {
        19
    }
    fn __nestrs_constructor_activate() -> usize {
        23
    }
}
fn injected<T: ?Sized>(_: &Injection<T>) {}
fn deferred<T: ?Sized>(_: &LazyInjection<T>) {}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    assert_eq!(OrdinaryBusinessService::__NESTRS_CONSTRUCTOR, 17);
    assert_eq!(
        OrdinaryBusinessService::__nestrs_constructor_dependencies(),
        19
    );
    assert_eq!(OrdinaryBusinessService::__nestrs_constructor_activate(), 23);
    let provider = ServiceProvider::build().await.unwrap();
    assert!(!ASYNC_READY.load(Ordering::SeqCst));
    let checkout = provider.get_required_service::<Checkout>().await.unwrap();
    let lazy_options = provider
        .get_required_service::<LazyOptions>()
        .await
        .unwrap();
    assert_eq!(checkout.label, "mail:0:700");
    injected::<Database>(&checkout.database);
    injected::<Clock>(&checkout.clock);
    injected::<dyn MailPort>(&checkout.mailer);
    injected::<Codec>(&checkout.codec);
    let _: &Option<Injection<Missing>> = &checkout.absent;
    deferred::<Reports>(&checkout.reports);
    assert!(checkout.absent.is_none());
    assert_eq!(REPORTS.load(Ordering::SeqCst), 0);
    deferred::<dyn MailPort>(&lazy_options.mailer);
    let _: &Option<LazyInjection<Missing>> = &lazy_options.absent;
    assert!(lazy_options.absent.is_none());
    assert_eq!(lazy_options.mailer.get().await.unwrap().name(), "mail");
    assert_eq!(checkout.reports.get().await.unwrap().id, 0);
    assert!(std::ptr::eq(
        checkout.reports.get().await.unwrap(),
        lazy_options.reports.as_ref().unwrap().get().await.unwrap(),
    ));
    assert_eq!(REPORTS.load(Ordering::SeqCst), 1);
    assert!(std::ptr::eq(
        &*checkout.clock,
        provider.get_required_service::<Clock>().await.unwrap()
    ));
    assert_eq!(
        provider
            .get_required_service::<ValidatedLabel>()
            .await
            .unwrap()
            .label,
        "clock:0"
    );
    let raw = provider.get_required_service::<RawService>().await.unwrap();
    injected::<Clock>(&raw.r#type);
    assert_eq!(raw.r#type.sequence(), 0);

    let repository = provider
        .get_required_service::<Repository<Customer>>()
        .await
        .unwrap();
    assert_eq!(repository.sequence(), 500);
    assert_eq!(repository.clock_sequence(), 0);
    let wrapper = provider
        .get_required_service::<GenericWrapper<Clock>>()
        .await
        .unwrap();
    assert!(std::ptr::eq(
        wrapper.get(),
        provider.get_required_service::<Clock>().await.unwrap(),
    ));
    assert_eq!(
        provider
            .get_required_service::<Outer<Order>>()
            .await
            .unwrap()
            .clock_sequence(),
        0
    );
    assert_eq!(
        provider
            .get_required_service::<dyn OuterPort>()
            .await
            .unwrap()
            .clock_sequence(),
        0
    );
    assert_eq!(
        provider
            .get_required_service::<dyn RepositoryPort>()
            .await
            .unwrap()
            .sequence(),
        500
    );
    assert_eq!(
        provider
            .get_required_service::<Configured>()
            .await
            .unwrap()
            .value,
        if cfg!(feature = "alternate") { 91 } else { 71 }
    );
    let generated = provider.get_required_service::<Generated>().await.unwrap();
    assert_eq!(generated.value, 33);
    assert_eq!(generated.clock.sequence(), 0);
    let external = provider
        .get_required_service::<ExternalService>()
        .await
        .unwrap();
    assert_eq!(external.value, 44);
    assert_eq!(external.clock.sequence(), 0);

    let first = provider.create_scope();
    let second = provider.create_scope();
    let request1 = first
        .service_provider()
        .get_required_service::<RequestService>()
        .await
        .unwrap();
    let request2 = second
        .service_provider()
        .get_required_service::<RequestService>()
        .await
        .unwrap();
    assert_ne!(request1.session.id, request2.session.id);
    assert_ne!(request1.first.id, request1.second.id);
    assert!(std::ptr::eq(
        request1,
        first
            .service_provider()
            .get_required_service::<RequestService>()
            .await
            .unwrap()
    ));
    assert_ne!(request1.first.id, request2.first.id);
    assert!(
        provider
            .get_required_service::<RequestService>()
            .await
            .is_err()
    );
    let ticket1 = provider.get_required_service::<Ticket>().await.unwrap();
    let ticket2 = provider.get_required_service::<Ticket>().await.unwrap();
    assert_ne!(ticket1.id, ticket2.id);
    first.dispose_async().await.unwrap();
    second.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    println!("constructor contracts passed");
}
