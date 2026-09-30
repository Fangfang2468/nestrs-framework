//! 真正经过编译器注册清单、图编译和 Tokio 调度的门面回归。
use nestrs::{factory, injectable, primary};
use nestrs_core::{InitializationMode, ServiceKey, ServiceProvider, ServiceProviderOptions};

use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static DATABASES: AtomicUsize = AtomicUsize::new(0);
static REQUESTS: AtomicUsize = AtomicUsize::new(0);
static TICKS: AtomicUsize = AtomicUsize::new(0);
static CLEANUPS: AtomicUsize = AtomicUsize::new(0);
struct Database {
    id: usize,
}
async fn cleanup_database() {
    CLEANUPS.fetch_add(1, Ordering::SeqCst);
}
#[factory(cleanup = "cleanup_database")]
async fn database() -> Database {
    tokio::task::yield_now().await;
    Database {
        id: DATABASES.fetch_add(1, Ordering::SeqCst),
    }
}
struct Request {
    id: usize,
    db: usize,
}
#[factory(lifetime = Scoped)]
async fn request(db: Database) -> Request {
    tokio::task::yield_now().await;
    Request {
        id: REQUESTS.fetch_add(1, Ordering::SeqCst),
        db: db.id,
    }
}
struct Tick {
    id: usize,
}
#[factory(lifetime = Transient)]
fn tick() -> Tick {
    Tick {
        id: TICKS.fetch_add(1, Ordering::SeqCst),
    }
}
#[injectable]
struct App {
    #[inject]
    database: Database,
    #[inject]
    first: Tick,
    #[inject]
    second: Tick,
    #[inject]
    absent: Option<Missing>,
    #[inject]
    absent_trait: Option<dyn MissingPort>,
}
struct Missing;
trait MissingPort: Send + Sync {}
#[injectable(lifetime = Transient)]
struct NeedsScope {
    #[inject]
    request: Request,
}
trait Greeting: Send + Sync {
    fn text(&self) -> &'static str;
}
#[injectable]
#[primary]
struct English;

impl Greeting for English {
    fn text(&self) -> &'static str {
        "hello"
    }
}
#[injectable]
struct French;

impl Greeting for French {
    fn text(&self) -> &'static str {
        "bonjour"
    }
}
#[injectable(key = "zh")]
struct Chinese;

impl Greeting for Chinese {
    fn text(&self) -> &'static str {
        "你好"
    }
}

struct User;
struct QueryOnly;
#[injectable]
struct Cache<T> {
    marker: PhantomData<T>,
    #[value(10)]
    value: usize,
}
#[injectable]
struct Repository<T> {
    #[inject]
    cache: Cache<T>,
}
#[factory]
fn custom_cache() -> Cache<User> {
    Cache {
        marker: PhantomData,
        value: 99,
    }
}

#[injectable(key = "named")]
struct KeyedCache<T> {
    marker: PhantomData<T>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn static_graph_full_lifecycle_and_frozen_routes() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(DATABASES.load(Ordering::SeqCst), 0);
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 0);
    let scope1 = provider.create_scope();
    let scope2 = provider.create_scope();
    assert_eq!(DATABASES.load(Ordering::SeqCst), 0);
    let (left, right) = tokio::join!(
        nestrs_core::get_required_service!(scope1.service_provider(), Database),
        nestrs_core::get_required_service!(scope2.service_provider(), Database)
    );
    assert!(std::ptr::eq(left.unwrap(), right.unwrap()));
    assert_eq!(DATABASES.load(Ordering::SeqCst), 1);
    let one = nestrs_core::get_required_service!(scope1.service_provider(), Request)
        .await
        .unwrap();
    let again = nestrs_core::get_required_service!(scope1.service_provider(), Request)
        .await
        .unwrap();
    let two = nestrs_core::get_required_service!(scope2.service_provider(), Request)
        .await
        .unwrap();
    assert!(std::ptr::eq(one, again));
    assert_ne!(one.id, two.id);
    assert_eq!(one.db, two.db);
    assert!(nestrs_core::get_service!(provider, Request).await.is_err());
    assert!(
        nestrs_core::get_service!(provider, NeedsScope)
            .await
            .is_err()
    );
    assert_eq!(
        nestrs_core::get_required_service!(scope1.service_provider(), NeedsScope)
            .await
            .unwrap()
            .request
            .id,
        one.id
    );
    let a = nestrs_core::get_required_service!(provider, Tick)
        .await
        .unwrap();
    let b = nestrs_core::get_required_service!(provider, Tick)
        .await
        .unwrap();
    assert_ne!(a.id, b.id);
    let app = nestrs_core::get_required_service!(provider, App)
        .await
        .unwrap();
    assert_ne!(app.first.id, app.second.id);
    assert_eq!(app.database.id, one.db);
    assert!(app.absent.is_none() && app.absent_trait.is_none());
    assert_eq!(
        nestrs_core::get_required_service!(provider, dyn Greeting)
            .await
            .unwrap()
            .text(),
        "hello"
    );
    assert_eq!(
        nestrs_core::get_required_keyed_service!(
            provider,
            dyn Greeting,
            ServiceKey::Named("zh".into())
        )
        .await
        .unwrap()
        .text(),
        "你好"
    );
    assert!(
        nestrs_core::get_service!(provider, Chinese)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        nestrs_core::get_required_service!(provider, Repository<User>)
            .await
            .unwrap()
            .cache
            .value,
        99
    );
    // The query macro contributes this closed type before build, although it is first queried here.
    assert_eq!(
        nestrs_core::get_service!(provider, Cache<QueryOnly>)
            .await
            .unwrap()
            .unwrap()
            .value,
        10
    );
    assert!(
        nestrs_core::get_service!(provider, KeyedCache<User>)
            .await
            .unwrap()
            .is_none()
    );
    nestrs_core::get_required_keyed_service!(
        provider,
        KeyedCache<User>,
        ServiceKey::Named("named".into())
    )
    .await
    .unwrap();
    scope1.warm_up().await.unwrap();
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 2);
    scope1.dispose_async().await.unwrap();
    scope2.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(CLEANUPS.load(Ordering::SeqCst), 1);

    let eager = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(DATABASES.load(Ordering::SeqCst), 2);
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 2);
    nestrs_core::get_required_service!(eager, App)
        .await
        .unwrap();
    assert_eq!(DATABASES.load(Ordering::SeqCst), 2);
    let scope = eager.create_scope();
    scope.warm_up().await.unwrap();
    assert_eq!(REQUESTS.load(Ordering::SeqCst), 3);
    scope.dispose_async().await.unwrap();
    eager.dispose_async().await.unwrap();
    assert_eq!(CLEANUPS.load(Ordering::SeqCst), 2);
}
