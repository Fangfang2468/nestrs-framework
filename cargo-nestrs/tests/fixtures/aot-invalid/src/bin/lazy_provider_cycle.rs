use nestrs::{injectable, lazy};

#[lazy(true)]
#[injectable]
struct DeferredAlpha {
    #[inject]
    beta: DeferredBeta,
}

#[injectable]
#[lazy]
struct DeferredBeta {
    #[inject]
    alpha: DeferredAlpha,
}

fn main() {
    panic!("must not execute");
}
