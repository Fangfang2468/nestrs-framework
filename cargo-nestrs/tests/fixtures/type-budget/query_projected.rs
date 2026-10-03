#![allow(dead_code)]

use nestrs_core::ServiceProvider;

include!("../wide.rs");

trait Family {
    type Target: Send + Sync + 'static;
}

struct Tag;

impl Family for Tag {
    type Target = Wide;
}

// The helper argument is small, but its actual service projection exceeds the
// DI budget. Deferring callable checks must not hide the normalized query type.
fn request<T: Family>(provider: &ServiceProvider) {
    drop(provider.get_service::<T::Target>());
}

fn never_called(provider: &ServiceProvider) {
    request::<Tag>(provider);
}

fn main() {}
