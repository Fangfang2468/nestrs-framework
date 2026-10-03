//! Suppression of an automatic pair must not hide user-declared duplicates.

use nestrs::{bind, factory, injectable};
use nestrs_core::ServiceProvider;
use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

trait Port: Send + Sync {}

#[injectable]
struct Service;

#[bind]
#[bind]
impl Port for Service {}

struct SideEffect;

#[factory]
fn side_effect() -> SideEffect {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    SideEffect
}

#[tokio::main]
async fn main() {
    let panic = tokio::spawn(ServiceProvider::build(None))
        .await
        .err()
        .expect("duplicate explicit bindings must still fail graph validation");
    assert!(panic.is_panic());
    let payload = panic.into_panic();
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("graph panic must include its diagnostic");
    assert!(message.contains("重复 trait binding"), "{message}");
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    println!("auto-binding duplicate explicit: rejected before any construction");
}
