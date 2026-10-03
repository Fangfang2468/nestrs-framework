use nestrs::{factory, injectable, lazy, primary};
#[lazy]
#[injectable]
#[lazy(false)]
struct OuterDuplicate;
#[injectable]
#[lazy(true)]
#[lazy(false)]
struct InnerDuplicate;
#[lazy(false)]
#[primary]
#[factory]
#[lazy(true)]
fn outer_factory() {}
#[factory]
#[lazy]
#[primary]
#[lazy]
fn inner_factory() {}
fn main() {}
