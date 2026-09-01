use nestrs_macro::constructor;

struct Service;

impl Service {
    #[constructor(unused)]
    fn new() -> Self {
        Self
    }
}

fn main() {}
