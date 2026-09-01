use nestrs_macro::constructor;

struct Service;

impl Service {
    #[constructor]
    unsafe fn new() -> Self {
        Service
    }
}

fn main() {}
