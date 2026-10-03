use nestrs::{injectable, lazy};

struct Missing;

#[lazy]
#[injectable]
struct DeferredUnused {
    #[inject]
    missing: Missing,
}

fn main() {
    panic!("must not execute");
}
