#![allow(dead_code)]

use nestrs_core::ServiceProvider;
use nestrs_di_regressions::{QueryFuture, Run, Start};
use std::marker::PhantomData;

#[nestrs::injectable]
struct Repo<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
    #[value(37)]
    value: usize,
}

struct QueryRunner<T>(PhantomData<T>);

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

impl<T: Send + Sync + 'static> Run for QueryRunner<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        query::<T>(provider)
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(<Start<u8> as Run>::run(&provider).await, 11);
    assert_eq!(<QueryRunner<u8> as Run>::run(&provider).await, 37);
    provider.dispose_async().await.unwrap();
}
