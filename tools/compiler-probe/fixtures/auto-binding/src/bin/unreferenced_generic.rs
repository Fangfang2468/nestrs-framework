//! A generic blueprint contributes dependencies only after materialization.

use nestrs::injectable;
use nestrs_core::ServiceProvider;
use std::marker::PhantomData;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

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
    let provider = ServiceProvider::build(None).await.unwrap();
    provider.get_required_service::<Concrete>().await.unwrap();
    assert_eq!(automatic_assertions::explicit_count(), 0);
    // This latent projection is available for downstream users. Its presence
    // must not turn the unused generic blueprint's field into a DI request.
    automatic_assertions::assert_count::<dyn Port>(1);
    provider.dispose_async().await.unwrap();
    println!("auto-binding unused generic: no unmaterialized trait request");
}
