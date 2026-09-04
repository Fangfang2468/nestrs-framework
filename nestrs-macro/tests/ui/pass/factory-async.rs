use nestrs_macro::factory;

#[factory]
async fn create() -> u8 {
    1
}

fn main() {}
