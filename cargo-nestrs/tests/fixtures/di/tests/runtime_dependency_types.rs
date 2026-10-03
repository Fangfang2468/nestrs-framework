//! Equivalent Rust types retain equivalent DI behavior after substitution.
use std::marker::PhantomData;

use nestrs::{factory, injectable};
use nestrs_core::ServiceProvider;

struct FactoryValue<T>(T);

#[factory]
fn factory_value() -> FactoryValue<u32> {
    FactoryValue(42)
}

#[factory(key = "named")]
fn named_factory_value() -> FactoryValue<u32> {
    FactoryValue(17)
}

#[injectable]
struct Repository<T: Default + Send + Sync + 'static> {
    value: T,
}

#[injectable]
struct Wrapper<T: Send + Sync + 'static + ?Sized> {
    #[inject]
    service: T,
}

type UserRepository = Repository<u32>;
type RepositoryAlias = UserRepository;

trait Store: Send + Sync {
    fn value(&self) -> u32;
}

#[injectable]
struct StoreImpl;

impl Store for StoreImpl {
    fn value(&self) -> u32 {
        73
    }
}

type StoreAlias = dyn Store;
trait Absent: Send + Sync {}
type AbsentAlias = dyn Absent;

#[injectable]
struct Consumer {
    #[inject]
    factory: FactoryValue<u32>,
    #[inject("named")]
    named: FactoryValue<u32>,
    #[inject]
    optional_factory: Option<FactoryValue<u32>>,
    #[inject]
    repository: RepositoryAlias,
    #[inject]
    store: StoreAlias,
    #[inject]
    optional_store: Option<StoreAlias>,
    #[inject]
    absent: Option<AbsentAlias>,
}

struct Combined(u32);

#[factory]
async fn combined(
    factory: FactoryValue<u32>,
    #[inject("named")] named: FactoryValue<u32>,
    repository: RepositoryAlias,
    store: StoreAlias,
    absent: Option<AbsentAlias>,
) -> Combined {
    tokio::task::yield_now().await;
    assert!(absent.is_none());
    Combined(factory.0 + named.0 + repository.value + store.value())
}

// This blueprint must stay passive: the closed impl is finite compiler input,
// but nothing in this link unit requests its interface or concrete service.
struct Unregistered;
trait DormantPort: Send + Sync {}
#[injectable]
struct Dormant<T: Send + Sync + 'static> {
    #[inject]
    _missing: Unregistered,
    _marker: PhantomData<T>,
}
impl DormantPort for Dormant<u32> {}

#[tokio::test]
async fn aliases_factory_only_types_and_substituted_parameters_resolve() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let consumer = provider.get_required_service::<Consumer>().await.unwrap();
    assert_eq!(consumer.factory.0, 42);
    assert_eq!(consumer.named.0, 17);
    assert_eq!(consumer.optional_factory.as_ref().unwrap().0, 42);
    assert_eq!(consumer.repository.value, 0);
    assert_eq!(consumer.store.value(), 73);
    assert_eq!(consumer.optional_store.as_ref().unwrap().value(), 73);
    assert!(consumer.absent.is_none());
    assert_eq!(
        provider.get_required_service::<Combined>().await.unwrap().0,
        132
    );

    // The macro can see only T in the generic body. The compiler's passive
    // closed blueprint directory supplies the recursively substituted types.
    let wrapped = provider
        .get_required_service::<Wrapper<Wrapper<Repository<u32>>>>()
        .await
        .unwrap();
    assert_eq!(wrapped.service.service.value, 0);
    assert_eq!(
        provider
            .get_required_service::<Wrapper<FactoryValue<u32>>>()
            .await
            .unwrap()
            .service
            .0,
        42,
    );
    assert_eq!(
        provider
            .get_required_service::<Wrapper<StoreAlias>>()
            .await
            .unwrap()
            .service
            .value(),
        73,
    );
    provider.dispose_async().await.unwrap();
}
