use nestrs::{factory, injectable};

use std::sync::atomic::{AtomicUsize, Ordering};
static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
struct Missing;
#[injectable]
struct UnusedBroken {
    #[inject]
    missing: Missing,
}
struct SideEffect;
#[factory]
fn side_effect() -> SideEffect {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    SideEffect
}

fn main() {}
