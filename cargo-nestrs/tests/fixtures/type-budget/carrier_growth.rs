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

// This never calls Run::run, but conservatively discovering a recursively
// growing carrier must terminate with DI008 before rustc normalization overflows.
fn grow<T>() {
    if false {
        grow::<Vec<T>>();
    }
}

fn main() {
    if false {
        grow::<Runner<u8>>();
    }
}
