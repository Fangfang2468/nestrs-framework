//! Query declarations and runtime arguments have separate lifetimes and responsibilities.
use nestrs::{factory, injectable};
use nestrs_core::{
    InitializationMode, ServiceKey, ServiceProvider, ServiceProviderOptions,
    get_required_service as required, get_service as optional,
};

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
    let _ = required!(provider, Repository<Dormant>).await;
    // A missing required query is still a runtime error, not a build-time required dependency.
    let _ = required!(provider, Missing).await;
}

mod shadowing {
    pub struct Option;
    pub struct Result;
    mod std {}

    pub async fn query(provider: &nestrs_core::ServiceProvider) -> usize {
        let _ = (Option, Result);
        nestrs_core::get_required_service!(provider, super::FactoryOnly<super::User>)
            .await
            .unwrap()
            .value
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn query_macros_seed_closed_roots_and_preserve_query_semantics() {
    let _ = never_executed;
    let provider_evaluations = AtomicUsize::new(0);
    let key_evaluations = AtomicUsize::new(0);
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(REPOSITORIES.load(Ordering::SeqCst), 0);
    assert_eq!(provider_evaluations.load(Ordering::SeqCst), 0);
    assert_eq!(key_evaluations.load(Ordering::SeqCst), 0);

    let (user, alias) = tokio::join!(
        required!(provider, Repository<User>),
        required!(provider, AliasRepository),
    );
    let user = user.unwrap();
    let alias = alias.unwrap();
    assert_ne!(user.serial, alias.serial);
    assert_eq!(REPOSITORIES.load(Ordering::SeqCst), 2);
    assert!(std::ptr::eq(
        user,
        required!(&provider, Repository<User>).await.unwrap(),
    ));

    assert_eq!(
        required!(provider, FactoryOnly<User>).await.unwrap().value,
        7
    );
    assert_eq!(required!(provider, FactoryAlias).await.unwrap().value, 7);
    assert_eq!(shadowing::query(&provider).await, 7);

    let keyed = nestrs_core::get_required_keyed_service!(
        provider_expression(&provider, &provider_evaluations),
        Keyed<User>,
        key_expression(&key_evaluations),
    )
    .await
    .unwrap();
    assert_eq!(keyed.value, 55);
    assert_eq!(provider_evaluations.load(Ordering::SeqCst), 1);
    assert_eq!(key_evaluations.load(Ordering::SeqCst), 1);
    assert!(optional!(provider, Keyed<User>).await.unwrap().is_none());
    let absent_key = ServiceKey::Named(format!("{}-absent", "replica"));
    assert!(
        nestrs_core::get_keyed_service!(provider, Keyed<User>, absent_key)
            .await
            .unwrap()
            .is_none()
    );

    assert!(optional!(provider, Missing).await.unwrap().is_none());
    assert!(
        optional!(provider, MissingGeneric<User>)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        optional!(provider, dyn MissingPort)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        required!(provider, Missing)
            .await
            .unwrap_err()
            .to_string()
            .contains("未注册")
    );

    let scope = provider.create_scope();
    let scoped = required!(scope.service_provider(), Scoped<User>)
        .await
        .unwrap();
    let view = scope.service_provider();
    assert!(std::ptr::eq(
        scoped,
        required!(view, Scoped<User>).await.unwrap()
    ));
    assert!(std::ptr::eq(
        scoped,
        required!(&view, Scoped<User>).await.unwrap()
    ));
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();

    let eager = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    })
    .await
    .unwrap();
    // Three Repository closures exist in the linked macro roots, including the never-called body.
    assert_eq!(REPOSITORIES.load(Ordering::SeqCst), 5);
    eager.dispose_async().await.unwrap();
}
