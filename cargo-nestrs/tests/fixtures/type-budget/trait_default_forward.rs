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

trait Step {
    fn step<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a>;
}

impl<T: Send + Sync + 'static> Step for QueryRunner<T> {
    fn step<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a> {
        query::<T>(provider)
    }
}

impl Step for Wide {
    fn step<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a> {
        empty(provider)
    }
}

trait Run: Step {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> QueryFuture<'a> {
        self.step(provider)
    }
}

impl<T: Send + Sync + 'static> Run for QueryRunner<T> {}
impl Run for Wide {}

fn never_called(value: &Wide, provider: &ServiceProvider) {
    drop(value.run(provider));
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(QueryRunner::<u8>(PhantomData).run(&provider).await, 37);
    provider.dispose_async().await.unwrap();
}
