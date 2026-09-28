//! Recursive class types must reach static graph validation, not overflow trait solving.
use nestrs::injectable;
use nestrs_core::ServiceProvider;

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

#[tokio::test]
async fn a_class_cycle_panics_during_graph_validation_before_any_construction() {
    let task = tokio::spawn(async {
        let _ = ServiceProvider::build().await;
    });
    let panic = task
        .await
        .expect_err("the whole class graph must be rejected")
        .into_panic();
    let diagnostic = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&'static str>().copied())
        .unwrap();
    assert!(diagnostic.contains("循环依赖"), "{diagnostic}");
    assert!(diagnostic.contains(".beta"), "{diagnostic}");
    assert!(diagnostic.contains(".alpha"), "{diagnostic}");
    assert_eq!(CONSTRUCTED.load(Ordering::SeqCst), 0);
}
