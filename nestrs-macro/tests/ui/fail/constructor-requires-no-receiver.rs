use nestrs_macro::constructor;

struct Service;

impl Service {
    #[constructor]
    fn new(&self) -> Self {
        Self
    }
}

fn main() {}
