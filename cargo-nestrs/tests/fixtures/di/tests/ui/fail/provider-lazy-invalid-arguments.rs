use nestrs::{factory, injectable, lazy};
#[lazy(1)]
#[injectable]
struct Number;
#[injectable]
#[lazy("true")]
struct StringValue;
#[lazy(enabled = true)]
#[factory]
fn named() {}
#[factory]
#[lazy(true, false)]
fn multiple() {}
fn main() {}
