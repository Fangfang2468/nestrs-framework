use nestrs_macro::constructor;

struct Service;

impl Service {
    #[constructor]
    async fn new() -> Self {
        Self
    }
}

fn main() {}
