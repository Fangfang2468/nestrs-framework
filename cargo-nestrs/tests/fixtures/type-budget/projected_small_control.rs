#![allow(dead_code)]

use nestrs_core::ServiceProvider;
use std::{future::Future, marker::PhantomData, pin::Pin};

include!("../wide.rs");

#[nestrs::injectable]
struct Repo<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
    #[value(37)]
    value: usize,
}

trait Family {
    type Target: Send + Sync + 'static;
}

impl Family for Wide {
    type Target = u8;
}

// The projection carries Wide before normalization, but the actual requested
// service is Repo<u8>. Its ordinary generic arguments must not consume a DI budget.
fn request<T: Family>(provider: &ServiceProvider) -> Pin<Box<dyn Future<Output = usize> + '_>> {
    Box::pin(async move {
        provider
            .get_required_service::<Repo<T::Target>>()
            .await
            .unwrap()
            .value
    })
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(request::<Wide>(&provider).await, 37);
    provider.dispose_async().await.unwrap();
}
