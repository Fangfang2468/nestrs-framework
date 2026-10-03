include!("../wide.rs");

#[nestrs::factory]
fn registered_type() -> Wide {
    panic!("graph analysis must not execute this factory")
}

fn main() {}
