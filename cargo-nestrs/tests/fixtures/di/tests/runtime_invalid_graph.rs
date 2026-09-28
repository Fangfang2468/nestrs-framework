use nestrs::{factory, injectable};
use nestrs_core::ServiceProvider;

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

#[tokio::test]
async fn unused_invalid_registration_panics_before_any_construction() {
    let failure = tokio::spawn(ServiceProvider::build())
        .await
        .err()
        .expect("graph must panic");
    assert!(failure.is_panic());
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
}
