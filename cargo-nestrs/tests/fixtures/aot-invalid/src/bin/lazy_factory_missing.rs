use nestrs::factory;
use nestrs_core::LazyInjection;
struct Missing;
struct LazyFactory {
    missing: LazyInjection<Missing>,
}
#[factory]
fn consumer(#[lazy] missing: Missing) -> LazyFactory {
    LazyFactory { missing }
}
fn main() {
    panic!("must not execute");
}
