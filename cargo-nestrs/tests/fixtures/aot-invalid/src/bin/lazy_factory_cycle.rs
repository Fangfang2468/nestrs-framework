use nestrs::factory;
use nestrs_core::LazyInjection;
struct First {
    second: LazyInjection<Second>,
}
struct Second {
    first: LazyInjection<First>,
}
#[factory]
fn first(#[lazy] second: Second) -> First {
    First { second }
}
#[factory]
fn second(#[lazy] first: First) -> Second {
    Second { first }
}
fn main() {
    panic!("must not execute");
}
