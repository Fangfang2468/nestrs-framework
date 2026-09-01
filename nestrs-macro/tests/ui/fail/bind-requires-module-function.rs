use nestrs_macro::bind;

fn main() {
    struct Service;

    #[bind]
    impl Service {}
}
