use nestrs_macro::factory;

#[factory]
fn value_parameter_factory(#[value(1)] dependency: u8) -> u8 {
    dependency
}

fn main() {}
