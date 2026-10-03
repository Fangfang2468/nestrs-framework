//! Recursive class types must reach static graph validation, not overflow trait solving.
use nestrs::injectable;

use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTED: AtomicUsize = AtomicUsize::new(0);

fn mark_constructed() {
    CONSTRUCTED.fetch_add(1, Ordering::SeqCst);
}

#[injectable]
#[allow(dead_code)]
struct Alpha {
    #[inject]
    beta: Beta,
    #[value(mark_constructed())]
    marker: (),
}

#[injectable]
#[allow(dead_code)]
struct Beta {
    #[inject]
    alpha: Alpha,
}

fn main() {}
