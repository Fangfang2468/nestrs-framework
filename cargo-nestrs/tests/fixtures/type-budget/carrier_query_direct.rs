#![allow(dead_code)]

use nestrs_core::ServiceProvider;
use std::marker::PhantomData;

include!("../wide.rs");

#[nestrs::injectable]
struct Repo<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
}

struct Runner<T>(PhantomData<T>);

impl<T> Runner<T> {
    fn new() -> Self {
        Self(PhantomData)
    }
}

trait Run {
    fn run(&self, provider: &ServiceProvider);
}

impl<T: Send + Sync + 'static> Run for Runner<T> {
    fn run(&self, provider: &ServiceProvider) {
        drop(provider.get_required_service::<Repo<T>>());
    }
}

// Compiled but unexecuted calls still create actual query roots.
fn never_called(provider: &ServiceProvider) {
    Runner::<Wide>::new().run(provider);
}

fn main() {}
