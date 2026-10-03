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
    const QUERY: for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a>;
}

impl<T: Send + Sync + 'static> Run for QueryRunner<T> {
    const QUERY: for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a> = query::<T>;
}

impl<T> Run for WideOf<T> {
    const QUERY: for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a> = empty;
}

impl<T> Run for Start<T> {
    const QUERY: for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a> = <WideOf<T> as Run>::QUERY;
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(<Start<u8> as Run>::QUERY(&provider).await, 11);
    assert_eq!(<QueryRunner<u8> as Run>::QUERY(&provider).await, 37);
    provider.dispose_async().await.unwrap();
}
