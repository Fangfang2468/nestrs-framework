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

// Independent instantiations are not a recursively growing discovery chain.
fn main() {
    let _small: Box<dyn Run> = Box::new(Runner::<u8>::new());
    let _wide: Box<dyn Run> = Box::new(Runner::<Wide>::new());
}
