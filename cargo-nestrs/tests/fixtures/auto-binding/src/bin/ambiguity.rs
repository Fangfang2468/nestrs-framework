//! Discovery must retain competing candidates and let graph validation reject them.

use nestrs::{factory, injectable};
use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

trait Port: Send + Sync {}

struct First;
struct Second;

impl Port for First {}
impl Port for Second {}

#[factory]
fn first() -> First {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    First
}

#[factory]
fn second() -> Second {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    Second
}

#[injectable]
struct Consumer {
    #[inject]
    _port: dyn Port,
}

fn main() {
    // Registration validation must reject this binary even without build().
    panic!("invalid auto-binding graph must not execute main");
}
