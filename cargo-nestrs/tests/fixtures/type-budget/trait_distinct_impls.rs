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
    fn run(provider: &ServiceProvider) -> QueryFuture<'_>;
}

impl<T: Send + Sync + 'static> Run for QueryRunner<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        query::<T>(provider)
    }
}

// These two actual method definitions are distinct. The trait declaration
// identity alone must not make this finite path look recursively growing.
impl<T> Run for WideOf<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        empty(provider)
    }
}

impl<T> Run for Start<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        <WideOf<T> as Run>::run(provider)
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(<Start<u8> as Run>::run(&provider).await, 11);
    assert_eq!(<QueryRunner<u8> as Run>::run(&provider).await, 37);
    provider.dispose_async().await.unwrap();
}
