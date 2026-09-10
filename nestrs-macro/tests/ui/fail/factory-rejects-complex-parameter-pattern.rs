use nestrs_macro::factory;

#[factory]
fn destructuring_factory((left, right): (u8, u8)) -> u8 {
    left + right
}

fn main() {}
