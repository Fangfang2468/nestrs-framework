use nestrs_macro::factory;

mod providers {
    use super::factory;

    #[factory]
    pub fn create() {}
}

fn main() {
    providers::create();
}
