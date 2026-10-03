//! A fixed small query must not disable protection of genuinely growing dispatch.
#![allow(dead_code)]
use nestrs_core::ServiceProvider;
use std::marker::PhantomData;

struct Repo<T>(PhantomData<T>);
struct Grow<T>(PhantomData<T>);

trait Run {
    fn run(provider: &ServiceProvider);
}

impl<T: Send + Sync + 'static> Run for Grow<T> {
    fn run(provider: &ServiceProvider) {
        drop(provider.get_service::<Repo<u8>>());
        if false {
            <Grow<(T,)> as Run>::run(provider);
        }
    }
}

fn never_called(provider: &ServiceProvider) {
    <Grow<u8> as Run>::run(provider);
}

fn main() {}
