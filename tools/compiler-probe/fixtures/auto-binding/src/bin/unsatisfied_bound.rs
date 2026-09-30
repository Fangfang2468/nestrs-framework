//! A syntactically matching generic impl must not bypass its compiler obligations.

use nestrs::injectable;
use nestrs_core::{ServiceProvider};
use std::marker::PhantomData;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

struct NoOrd;

trait Port: Send + Sync {}

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
}

impl<T: Ord + Send + Sync> Port for Repository<T> {}

type Closed = Repository<NoOrd>;

#[injectable]
struct OptionalConsumer {
    #[inject]
    port: Option<dyn Port>,
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    nestrs_core::get_required_service!(provider, Closed)
        .await
        .unwrap();
    let consumer = nestrs_core::get_required_service!(provider, OptionalConsumer)
        .await
        .unwrap();
    assert!(consumer.port.is_none());
    assert!(
        nestrs_core::get_service!(provider, dyn Port)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(automatic_assertions::explicit_count(), 0);
    automatic_assertions::assert_count::<dyn Port>(0);
    provider.dispose_async().await.unwrap();
    println!("auto-binding unsatisfied bound: no invalid candidate generated");
}
