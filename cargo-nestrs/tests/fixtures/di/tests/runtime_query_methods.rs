//! 普通查询方法的闭合类型由编译器收集，provider 与动态 key 表达式只在运行时求值一次。
use nestrs::{factory, injectable};
use nestrs_core::{InitializationMode, ServiceKey, ServiceProvider, ServiceProviderOptions};

use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static REPOSITORIES: AtomicUsize = AtomicUsize::new(0);

struct User;
struct AliasOnly;
struct Dormant;

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
    #[value(REPOSITORIES.fetch_add(1, Ordering::SeqCst))]
    serial: usize,
}
type AliasRepository = Repository<AliasOnly>;

#[injectable(key = "replica")]
struct Keyed<T> {
    marker: PhantomData<T>,
    #[value(55)]
    value: usize,
}

#[injectable(lifetime = Scoped)]
struct Scoped<T> {
    marker: PhantomData<T>,
}

struct FactoryOnly<T> {
    marker: PhantomData<T>,
    value: usize,
}
type FactoryAlias = FactoryOnly<User>;

#[factory]
async fn factory_only() -> FactoryOnly<User> {
    tokio::task::yield_now().await;
    FactoryOnly {
        marker: PhantomData,
        value: 7,
    }
}

#[derive(Debug)]
struct Missing;
struct MissingGeneric<T>(PhantomData<T>);
trait MissingPort: Send + Sync {}

fn provider_expression<'provider>(
    provider: &'provider ServiceProvider,
    evaluations: &AtomicUsize,
) -> &'provider ServiceProvider {
    evaluations.fetch_add(1, Ordering::SeqCst);
    provider
}

fn key_expression(evaluations: &AtomicUsize) -> ServiceKey {
    evaluations.fetch_add(1, Ordering::SeqCst);
    ServiceKey::Named(String::from("replica"))
}

async fn never_executed(provider: &ServiceProvider) {
    let _ = provider.get_required_service::<Repository<Dormant>>().await;
    // A missing required query is still a runtime error, not a build-time required dependency.
    let _ = provider.get_required_service::<Missing>().await;
}

mod shadowing {
    pub struct Option;
    pub struct Result;
    mod std {}

    pub async fn query(provider: &nestrs_core::ServiceProvider) -> usize {
        let _ = (Option, Result);
        provider
            .get_required_service::<super::FactoryOnly<super::User>>()
            .await
            .unwrap()
            .value
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_methods_collect_closed_roots_and_preserve_query_semantics() {
    let _ = never_executed;
    let provider_evaluations = AtomicUsize::new(0);
    let key_evaluations = AtomicUsize::new(0);
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(REPOSITORIES.load(Ordering::SeqCst), 0);
    assert_eq!(provider_evaluations.load(Ordering::SeqCst), 0);
    assert_eq!(key_evaluations.load(Ordering::SeqCst), 0);

    let (user, alias) = tokio::join!(
        provider.get_required_service::<Repository<User>>(),
        provider.get_required_service::<AliasRepository>(),
    );
    let user = user.unwrap();
    let alias = alias.unwrap();
    assert_ne!(user.serial, alias.serial);
    assert_eq!(REPOSITORIES.load(Ordering::SeqCst), 2);
    assert!(std::ptr::eq(
        user,
        (&provider)
            .get_required_service::<Repository<User>>()
            .await
            .unwrap(),
    ));

    assert_eq!(
        provider
            .get_required_service::<FactoryOnly<User>>()
            .await
            .unwrap()
            .value,
        7
    );
    assert_eq!(
        provider
            .get_required_service::<FactoryAlias>()
            .await
            .unwrap()
            .value,
        7
    );
    assert_eq!(shadowing::query(&provider).await, 7);

    let keyed = provider_expression(&provider, &provider_evaluations)
        .get_required_keyed_service::<Keyed<User>>(key_expression(&key_evaluations))
        .await
        .unwrap();
    assert_eq!(keyed.value, 55);
    assert_eq!(provider_evaluations.load(Ordering::SeqCst), 1);
    assert_eq!(key_evaluations.load(Ordering::SeqCst), 1);
    assert!(
        provider
            .get_service::<Keyed<User>>()
            .await
            .unwrap()
            .is_none()
    );
    let absent_key = ServiceKey::Named(format!("{}-absent", "replica"));
    assert!(
        provider
            .get_keyed_service::<Keyed<User>>(absent_key)
            .await
            .unwrap()
            .is_none()
    );

    assert!(provider.get_service::<Missing>().await.unwrap().is_none());
    assert!(
        provider
            .get_service::<MissingGeneric<User>>()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        provider
            .get_service::<dyn MissingPort>()
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        provider
            .get_required_service::<Missing>()
            .await
            .unwrap_err()
            .to_string()
            .contains("未注册")
    );

    let scope = provider.create_scope(None).await.unwrap();
    let scoped = scope
        .service_provider()
        .get_required_service::<Scoped<User>>()
        .await
        .unwrap();
    let view = scope.service_provider();
    assert!(std::ptr::eq(
        scoped,
        view.get_required_service::<Scoped<User>>().await.unwrap()
    ));
    assert!(std::ptr::eq(
        scoped,
        (&view)
            .get_required_service::<Scoped<User>>()
            .await
            .unwrap()
    ));
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();

    let eager = ServiceProvider::build(Some(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    }))
    .await
    .unwrap();
    // 编译器收集三个 Repository 闭合根，包括未调用函数中的查询。
    assert_eq!(REPOSITORIES.load(Ordering::SeqCst), 5);
    eager.dispose_async().await.unwrap();
}
