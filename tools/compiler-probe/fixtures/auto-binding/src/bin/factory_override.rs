//! Exact type/key factories suppress the unused generic blueprint's demands.

use nestrs::{factory, injectable};
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceKey, ServiceProvider};
use std::marker::PhantomData;

struct User;

trait Port: Send + Sync {}

#[injectable]
struct Concrete;

impl Port for Concrete {}

#[injectable]
struct DefaultRepository<T> {
    marker: PhantomData<T>,
    #[inject]
    port: Option<dyn Port>,
}

#[injectable(key = "same")]
struct NamedRepository<T> {
    marker: PhantomData<T>,
    #[inject]
    port: Option<dyn Port>,
}

#[injectable(key = 7)]
struct IndexedRepository<T> {
    marker: PhantomData<T>,
    #[inject]
    port: Option<dyn Port>,
}

#[factory]
fn default_repository() -> DefaultRepository<User> {
    DefaultRepository {
        marker: PhantomData,
        port: None,
    }
}

#[factory(key = "same")]
fn named_repository() -> NamedRepository<User> {
    NamedRepository {
        marker: PhantomData,
        port: None,
    }
}

#[factory(key = 7)]
fn indexed_repository() -> IndexedRepository<User> {
    IndexedRepository {
        marker: PhantomData,
        port: None,
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(REFLECTED_BINDINGS.len(), 0);
    let default = nestrs_core::get_required_service!(provider, DefaultRepository<User>)
        .await
        .unwrap();
    let named = nestrs_core::get_required_keyed_service!(
        provider,
        NamedRepository<User>,
        ServiceKey::Named("same".into())
    )
    .await
    .unwrap();
    let indexed = nestrs_core::get_required_keyed_service!(
        provider,
        IndexedRepository<User>,
        ServiceKey::Indexed(7)
    )
    .await
    .unwrap();
    assert!(default.port.is_none());
    assert!(named.port.is_none());
    assert!(indexed.port.is_none());
    provider.dispose_async().await.unwrap();
    println!("auto-binding factory override: default/named/indexed blueprints remain unused");
}
