//! A syntactically matching generic impl must not bypass its compiler obligations.

use nestrs::injectable;
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceProvider};
use std::marker::PhantomData;

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
    assert_eq!(REFLECTED_BINDINGS.len(), 0);
    provider.dispose_async().await.unwrap();
    println!("auto-binding unsatisfied bound: no invalid candidate generated");
}
