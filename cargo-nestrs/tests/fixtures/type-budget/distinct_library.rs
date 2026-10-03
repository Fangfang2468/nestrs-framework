//! Cross-crate actual implementation identities used by the budget contract.
use nestrs_core::ServiceProvider;
use std::{future::Future, marker::PhantomData, pin::Pin};

include!("wide_generic.rs");

pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = usize> + 'a>>;

pub trait Run {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_>;
}

pub struct Start<T>(PhantomData<T>);

impl<T> Run for WideOf<T> {
    fn run(_: &ServiceProvider) -> QueryFuture<'_> {
        Box::pin(async { 11 })
    }
}

impl<T> Run for Start<T> {
    fn run(provider: &ServiceProvider) -> QueryFuture<'_> {
        <WideOf<T> as Run>::run(provider)
    }
}
