//! A generic blueprint contributes dependencies only after materialization.

use nestrs::injectable;
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceProvider};
use std::marker::PhantomData;

trait Port: Send + Sync {}

#[injectable]
struct Concrete;

impl Port for Concrete {}

#[allow(dead_code)]
#[injectable]
struct Unused<T> {
    marker: PhantomData<T>,
    #[inject]
    port: dyn Port,
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    nestrs_core::get_required_service!(provider, Concrete)
        .await
        .unwrap();
    assert_eq!(REFLECTED_BINDINGS.len(), 0);
    provider.dispose_async().await.unwrap();
    println!("auto-binding unused generic: no unmaterialized trait request");
}
