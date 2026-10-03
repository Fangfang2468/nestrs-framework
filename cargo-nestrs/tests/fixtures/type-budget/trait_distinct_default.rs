#![allow(dead_code)]

use nestrs_core::ServiceProvider;
use std::{future::Future, marker::PhantomData, pin::Pin};

include!("../wide_generic.rs");

type QueryFuture<'a> = Pin<Box<dyn Future<Output = usize> + 'a>>;

#[nestrs::injectable]
struct Repo<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
    #[value(37)]
    value: usize,
}

struct QueryRunner<T>(PhantomData<T>);
struct Start<T>(PhantomData<T>);

fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    Box::pin(async move {
        provider
            .get_required_service::<Repo<T>>()
            .await
            .unwrap()
            .value
    })
}

fn empty(_: &ServiceProvider) -> QueryFuture<'_> {
    Box::pin(async { 11 })
}

trait Run {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        empty(provider)
    }
}

impl<T: Send + Sync + 'static> Run for QueryRunner<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        query::<T>(provider)
    }
}

// The terminal implementation inherits the trait default, while Start's
// implementation has its own method. This is not repeated actual dispatch.
impl<T> Run for WideOf<T> {}

impl<T> Run for Start<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        <WideOf<T> as Run>::run(provider)
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(<Start<u8> as Run>::run(&provider).await, 11);
    assert_eq!(<QueryRunner<u8> as Run>::run(&provider).await, 37);
    provider.dispose_async().await.unwrap();
}
