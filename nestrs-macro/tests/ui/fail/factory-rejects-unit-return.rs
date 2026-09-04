use nestrs_macro::factory;

#[factory]
fn implicit_unit() {}

#[factory]
fn explicit_unit() -> () {}

#[factory]
fn parenthesized_unit() -> (()) {}

fn main() {}
