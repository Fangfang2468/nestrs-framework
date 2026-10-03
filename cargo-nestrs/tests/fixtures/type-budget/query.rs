#![allow(dead_code)]

include!("../wide.rs");

fn never_called(provider: &nestrs_core::ServiceProvider) {
    drop(provider.get_service::<Wide>());
}

fn main() {}
