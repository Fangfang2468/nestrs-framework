use nestrs_macro::constructor;

mod providers {
    use super::constructor;

    pub struct Service;

    impl Service {
        #[constructor]
        pub fn new() -> Self {
            Self
        }
    }
}

fn main() {
    providers::Service::new();
}
