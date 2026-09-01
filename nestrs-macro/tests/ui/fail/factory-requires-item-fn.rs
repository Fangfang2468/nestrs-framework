use nestrs_macro::factory;

struct Factory;

impl Factory {
    #[factory]
    fn create(&self) {}
}

fn main() {}
