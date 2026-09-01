use nestrs_macro::factory;

struct Factory;

impl Factory {
    #[factory]
    fn create() -> u32 {
        1
    }
}

fn main() {}
