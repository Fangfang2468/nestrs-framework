//! Discovery must retain competing candidates and let graph validation reject them.

use nestrs::{factory, injectable};
use nestrs_core::ServiceProvider;
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

#[tokio::main]
async fn main() {
    let panic = tokio::spawn(ServiceProvider::build())
        .await
        .err()
        .expect("two automatic candidates must fail graph validation");
    assert!(panic.is_panic());
    let payload = panic.into_panic();
    let message = payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .expect("graph panic must include its diagnostic");
    assert!(message.contains("trait 候选不唯一"), "{message}");
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    println!("auto-binding ambiguity: rejected before any construction");
}
