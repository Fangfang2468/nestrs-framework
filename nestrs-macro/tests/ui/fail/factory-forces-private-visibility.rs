use nestrs_macro::factory;

mod providers {
    use super::factory;

    #[factory]
    pub fn create() -> u8 {
        1
    }
}

fn main() {
    providers::create();
}
