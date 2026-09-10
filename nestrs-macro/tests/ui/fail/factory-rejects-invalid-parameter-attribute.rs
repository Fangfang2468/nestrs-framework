use nestrs_macro::factory;

#[factory]
fn attributed_parameter_factory(#[allow(unused)] dependency: u8) -> u8 {
    dependency
}

fn main() {}
