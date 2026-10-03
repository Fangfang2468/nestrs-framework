//! A growing associated-constant path must retain the finite query budget.
#![allow(dead_code)]
use nestrs_core::ServiceProvider;
use std::marker::PhantomData;

struct Repo<T>(PhantomData<T>);
struct Grow<T>(PhantomData<T>);

trait Run {
    const QUERY: fn(&ServiceProvider);
}

impl<T: Send + Sync + 'static> Run for Grow<T> {
    const QUERY: fn(&ServiceProvider) = growing::<T>;
}

fn growing<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    drop(provider.get_service::<Repo<u8>>());
    if false {
        <Grow<(T,)> as Run>::QUERY(provider);
    }
}

fn never_called(provider: &ServiceProvider) {
    <Grow<u8> as Run>::QUERY(provider);
}

fn main() {}
