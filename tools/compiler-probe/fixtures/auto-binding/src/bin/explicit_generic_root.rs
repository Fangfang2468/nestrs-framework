//! An explicit closed binding is a graph root even if never queried.

use nestrs::{bind, injectable};
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceProvider};
use std::marker::PhantomData;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

struct User;

trait DependencyPort: Send + Sync {}

#[injectable]
struct Dependency;

impl DependencyPort for Dependency {}

trait UnqueriedPort: Send + Sync {}

#[allow(dead_code)]
#[injectable]
struct Generic<T> {
    marker: PhantomData<T>,
    #[inject]
    dependency: dyn DependencyPort,
}

#[bind]
impl UnqueriedPort for Generic<User> {}

#[tokio::main]
async fn main() {
    // The existing binding materializes Generic<User> and validates all its
    // dependencies. There is deliberately no query for either interface or
    // the closed generic, so only correct binding-root discovery can supply
    // the DependencyPort demand for automatic binding.
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(REFLECTED_BINDINGS.len(), 1);
    automatic_assertions::assert_count::<dyn DependencyPort>(1);
    automatic_assertions::assert_count::<dyn UnqueriedPort>(0);
    provider.dispose_async().await.unwrap();
    println!("auto-binding explicit generic root: unqueried blueprint dependency bound");
}
