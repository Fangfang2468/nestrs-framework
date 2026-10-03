#![allow(dead_code)]

use std::marker::PhantomData;

include!("../wide.rs");

#[nestrs::injectable]
struct Service<T> {
    marker: PhantomData<T>,
}

trait Family {
    type Item;
}
struct Tag;
impl Family for Tag {
    type Item = Service<Wide>;
}
trait Ordinary {}
impl Ordinary for <Tag as Family>::Item {}

fn main() {}
