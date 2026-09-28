//! A factory for another key must not suppress the default generic blueprint.

use nestrs::{factory, injectable};
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceKey, ServiceProvider};
use std::marker::PhantomData;

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
    assert_eq!(REFLECTED_BINDINGS.len(), 1);
    let default = nestrs_core::get_required_service!(provider, Repository<User>)
        .await
        .unwrap();
    let keyed = nestrs_core::get_required_keyed_service!(
        provider,
        Repository<User>,
        ServiceKey::Named("factory".into())
    )
    .await
    .unwrap();
    assert_eq!(default.port.as_ref().unwrap().value(), 41);
    assert!(keyed.port.is_none());
    assert!(!std::ptr::eq(default, keyed));
    provider.dispose_async().await.unwrap();
    println!("auto-binding factory other key: generic default blueprint remains active");
}
