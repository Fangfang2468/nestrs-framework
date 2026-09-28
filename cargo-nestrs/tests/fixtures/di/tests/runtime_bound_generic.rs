//! Only dyn-trait queries are present: bind supplies every generic concrete anchor.
use nestrs::{bind, factory, injectable};
use std::marker::PhantomData;

use nestrs_core::{
    __private::{REFLECTED_BINDINGS, REFLECTED_ROOTS, ServiceType},
    ServiceKey, ServiceProvider, get_required_keyed_service, get_required_service, get_service,
};

struct User;
struct Order;

#[injectable]
struct Cache<T> {
    marker: PhantomData<T>,
    #[value(41)]
    value: usize,
}

#[injectable]
struct Repository<T> {
    #[inject]
    cache: Cache<T>,
}

trait Port: Send + Sync {
    fn value(&self) -> usize;
    fn identity(&self) -> usize;
}

#[bind]
impl Port for Repository<User> {
    fn value(&self) -> usize {
        self.cache.value
    }
    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

type RepositoryAlias = Repository<User>;
trait AliasPort: Send + Sync {
    fn identity(&self) -> usize;
}

#[bind]
impl AliasPort for RepositoryAlias {
    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

struct FactoryOnly<T> {
    marker: PhantomData<T>,
    value: usize,
}
type FactoryAlias = FactoryOnly<User>;
trait FactoryPort: Send + Sync {
    fn value(&self) -> usize;
}

#[factory(key = "factory")]
fn make_factory_only() -> FactoryOnly<User> {
    FactoryOnly {
        marker: PhantomData,
        value: 73,
    }
}

#[bind]
impl FactoryPort for FactoryAlias {
    fn value(&self) -> usize {
        self.value
    }
}

#[injectable(key = "named")]
struct Keyed<T> {
    marker: PhantomData<T>,
    #[value(91)]
    value: usize,
}
trait KeyedPort: Send + Sync {
    fn value(&self) -> usize;
}

#[bind]
impl KeyedPort for Keyed<Order> {
    fn value(&self) -> usize {
        self.value
    }
}

#[tokio::test]
async fn trait_only_queries_materialize_closed_bind_targets_and_preserve_factory_fallbacks() {
    let roots: Vec<_> = REFLECTED_ROOTS.iter().map(|declare| declare()).collect();
    assert!(!roots.is_empty());
    assert!(roots.iter().all(|root| root.materialize.is_none()));
    assert!(roots.iter().all(|root| {
        [
            ServiceType::create::<dyn Port>(),
            ServiceType::create::<dyn AliasPort>(),
            ServiceType::create::<dyn FactoryPort>(),
            ServiceType::create::<dyn KeyedPort>(),
        ]
        .contains(&root.service_type)
    }));
    let bindings: Vec<_> = REFLECTED_BINDINGS.iter().map(|declare| declare()).collect();
    assert!(
        bindings
            .iter()
            .find(|binding| binding.concrete_type == ServiceType::create::<FactoryAlias>())
            .unwrap()
            .materialize
            .is_none()
    );
    assert_eq!(
        bindings
            .iter()
            .filter(|binding| binding.materialize.is_some())
            .count(),
        3
    );

    let provider = ServiceProvider::build().await.unwrap();
    let repository = get_required_service!(provider, dyn Port).await.unwrap();
    assert_eq!(repository.value(), 41);
    let alias = get_required_service!(provider, dyn AliasPort)
        .await
        .unwrap();
    assert_eq!(
        repository.identity(),
        alias.identity(),
        "both bindings must share one concrete singleton"
    );
    let factory = get_required_keyed_service!(
        provider,
        dyn FactoryPort,
        ServiceKey::Named("factory".into())
    )
    .await
    .unwrap();
    assert_eq!(factory.value(), 73);
    assert!(
        get_service!(provider, dyn FactoryPort)
            .await
            .unwrap()
            .is_none()
    );
    let keyed =
        get_required_keyed_service!(provider, dyn KeyedPort, ServiceKey::Named("named".into()))
            .await
            .unwrap();
    assert_eq!(keyed.value(), 91);
    assert!(
        get_service!(provider, dyn KeyedPort)
            .await
            .unwrap()
            .is_none()
    );
    provider.dispose_async().await.unwrap();
}
