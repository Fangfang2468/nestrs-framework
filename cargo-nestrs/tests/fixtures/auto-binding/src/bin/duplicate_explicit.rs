//! Suppression of an automatic pair must not hide user-declared duplicates.

use nestrs::{bind, factory, injectable};
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

fn main() {
    // Duplicate explicit declarations are a compile-time error, not a panic.
    panic!("invalid auto-binding graph must not execute main");
}
