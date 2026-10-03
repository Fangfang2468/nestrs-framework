#![allow(dead_code)]

use std::marker::PhantomData;

include!("../wide.rs");

trait OrdinaryTuple {}
impl OrdinaryTuple for Wide {}

struct Wrapper<T>(PhantomData<T>);
trait OrdinaryGeneric {}
impl OrdinaryGeneric for Wrapper<Wide> {}

trait Family {
    type Item;
}
struct Tag;
impl Family for Tag {
    type Item = Wrapper<Wide>;
}
trait OrdinaryProjection {}
impl OrdinaryProjection for <Tag as Family>::Item {}

// The presence of real DI declarations must not make unrelated impls consume
// the DI budget. The default configuration also compiles through plain Cargo.
#[cfg(feature = "with-di")]
mod services {
    #[nestrs::injectable]
    struct Service;

    fn never_called(provider: &nestrs_core::ServiceProvider) {
        drop(provider.get_service::<Service>());
    }
}

fn main() {
    assert_eq!(std::mem::size_of::<Wide>(), (0..1100).sum::<usize>());
}
