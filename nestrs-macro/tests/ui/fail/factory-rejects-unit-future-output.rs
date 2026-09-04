#[allow(unused_imports)]
use core::future::Future;
use nestrs_macro::factory;

#[factory]
fn bare_future_factory() -> impl Future<Output = ()> {
    async {}
}

#[factory]
fn std_future_factory() -> impl ::std::future::Future<Output = ()> {
    async {}
}

#[factory]
fn core_future_factory() -> impl ::core::future::Future<Output = (())> + Send {
    async {}
}

fn main() {}
