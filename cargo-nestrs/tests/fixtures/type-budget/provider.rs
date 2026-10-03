#![allow(dead_code)]

use std::marker::PhantomData;

include!("../wide.rs");

#[nestrs::injectable]
struct Service<T> {
    marker: PhantomData<T>,
}

trait Ordinary {}
impl Ordinary for Service<Wide> {}

fn main() {}
