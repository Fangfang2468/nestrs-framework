use nestrs_macro::constructor;

struct Service;

impl Service {
    #[constructor]
    fn new() -> Self {
        Self
    }

    #[constructor]
    fn from_explicit_type() -> Service {
        Service
    }
}

fn main() {
    let _ = Service::new();
    let _ = Service::from_explicit_type();
}
