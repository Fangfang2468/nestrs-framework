use nestrs_macro::constructor;

struct Service;

impl Service {
    #[constructor]
    extern "C" fn new() -> Self {
        Service
    }
}

fn main() {}
