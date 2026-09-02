use nestrs_macro::bind;

trait ServiceInterface: Send + Sync {}

fn main() {
    struct Service;

    #[bind]
    impl ServiceInterface for Service {}
}
