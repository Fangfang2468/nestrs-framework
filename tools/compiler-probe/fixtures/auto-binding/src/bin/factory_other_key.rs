//! A factory for another key must not suppress the default generic blueprint.

use nestrs::{factory, injectable};
use nestrs_core::{ServiceKey, ServiceProvider};
use std::marker::PhantomData;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

struct User;

trait Port: Send + Sync {
    fn value(&self) -> usize;
}

#[injectable]
struct Concrete;

impl Port for Concrete {
    fn value(&self) -> usize {
        41
    }
}

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
    #[inject]
    port: Option<dyn Port>,
}

#[factory(key = "factory")]
fn factory_repository() -> Repository<User> {
    Repository {
        marker: PhantomData,
        port: None,
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(automatic_assertions::explicit_count(), 0);
    automatic_assertions::assert_count::<dyn Port>(1);
    let default = provider
        .get_required_service::<Repository<User>>()
        .await
        .unwrap();
    let keyed = provider
        .get_required_keyed_service::<Repository<User>>(ServiceKey::Named("factory".into()))
        .await
        .unwrap();
    assert_eq!(default.port.as_ref().unwrap().value(), 41);
    assert!(keyed.port.is_none());
    assert!(!std::ptr::eq(default, keyed));
    provider.dispose_async().await.unwrap();
    println!("auto-binding factory other key: generic default blueprint remains active");
}
