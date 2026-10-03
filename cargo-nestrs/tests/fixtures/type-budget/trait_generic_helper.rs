#![allow(dead_code)]

use nestrs_core::ServiceProvider;
use std::{future::Future, marker::PhantomData, pin::Pin};

include!("../wide.rs");

type QueryFuture<'a> = Pin<Box<dyn Future<Output = usize> + 'a>>;

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

trait Run {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a>;
}

impl<T: Send + Sync + 'static> Run for QueryRunner<T> {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a> {
        query::<T>(provider)
    }
}

impl Run for Wide {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a> {
        empty(provider)
    }
}

fn invoke<'a, T: Run>(value: &T, provider: &'a ServiceProvider) -> QueryFuture<'a> {
    value.run(provider)
}

// Relevance of invoke<QueryRunner<u8>> must not make invoke<Wide> a DI type.
fn never_called(value: &Wide, provider: &ServiceProvider) {
    drop(invoke(value, provider));
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(invoke(&QueryRunner::<u8>(PhantomData), &provider).await, 37);
    provider.dispose_async().await.unwrap();
}
