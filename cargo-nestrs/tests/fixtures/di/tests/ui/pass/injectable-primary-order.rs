#![deny(unused_imports)]
mod primary_before_injectable {
    use nestrs::{injectable, primary};

    #[primary]
    #[injectable]
    struct Service;
}

mod injectable_before_primary {
    use nestrs::{injectable, primary};

    #[injectable]
    #[primary()]
    struct Service;
}

fn main() {}
