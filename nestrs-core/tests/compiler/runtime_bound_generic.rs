//! Only dyn-trait queries are present: bind supplies every generic concrete anchor.
use crate::service::ServiceType;
use nestrs::{bind, factory, injectable};
use std::marker::PhantomData;

use nestrs_core::{ServiceKey, ServiceProvider};

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
    // 最终 reflect 计划已经完成泛型闭合，factory-only 也直接提供同一执行契约。
    let graph = &crate::graph::plan::CompiledApplication::load().graph;
    for service_type in [
        ServiceType::create::<Repository<User>>(),
        ServiceType::create::<Cache<User>>(),
        ServiceType::create::<FactoryAlias>(),
        ServiceType::create::<Keyed<Order>>(),
    ] {
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.identifier.service_type == service_type)
                .count(),
            1
        );
    }
    assert_eq!(graph.nodes.len(), 4);

    let provider = ServiceProvider::build(None).await.unwrap();
    let repository = provider.get_required_service::<dyn Port>().await.unwrap();
    assert_eq!(repository.value(), 41);
    let alias = provider
        .get_required_service::<dyn AliasPort>()
        .await
        .unwrap();
    assert_eq!(
        repository.identity(),
        alias.identity(),
        "both bindings must share one concrete singleton"
    );
    let factory = provider
        .get_required_keyed_service::<dyn FactoryPort>(ServiceKey::Named("factory".into()))
        .await
        .unwrap();
    assert_eq!(factory.value(), 73);
    assert!(
        provider
            .get_service::<dyn FactoryPort>()
            .await
            .unwrap()
            .is_none()
    );
    let keyed = provider
        .get_required_keyed_service::<dyn KeyedPort>(ServiceKey::Named("named".into()))
        .await
        .unwrap();
    assert_eq!(keyed.value(), 91);
    assert!(
        provider
            .get_service::<dyn KeyedPort>()
            .await
            .unwrap()
            .is_none()
    );
    provider.dispose_async().await.unwrap();
}
