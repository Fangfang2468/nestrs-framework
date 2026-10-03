//! Graph inspection is independent of container ownership and fallible external resources.
use nestrs::factory;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use nestrs_core::{BuildError, InitializationMode, ServiceProvider, ServiceProviderOptions};

static SHOULD_FAIL: AtomicBool = AtomicBool::new(false);
static ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static CLEANUPS: AtomicUsize = AtomicUsize::new(0);
static DROPS: AtomicUsize = AtomicUsize::new(0);

struct Resource;

impl Drop for Resource {
    fn drop(&mut self) {
        let previous = DROPS.fetch_add(1, Ordering::SeqCst);
        assert!(CLEANUPS.load(Ordering::SeqCst) > previous);
    }
}

async fn cleanup_resource() {
    // Force a suspension so merely starting cleanup cannot satisfy the assertions below.
    tokio::task::yield_now().await;
    CLEANUPS.fetch_add(1, Ordering::SeqCst);
}

#[factory(cleanup = "cleanup_resource")]
async fn resource() -> Result<Resource, &'static str> {
    ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    tokio::task::yield_now().await;
    if SHOULD_FAIL.load(Ordering::SeqCst) {
        Err("graph snapshot initialization sentinel")
    } else {
        CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
        Ok(Resource)
    }
}

#[test]
fn graph_snapshot_is_independent_of_runtime_and_external_initialization() {
    // Static diagnostics work before any Tokio runtime or container is created.
    let graph =
        crate::graph::snapshot(&crate::graph::plan::CompiledApplication::load().graph).to_string();
    assert!(graph.contains("Resource"));
    assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 0);
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    assert_eq!(CLEANUPS.load(Ordering::SeqCst), 0);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let lazy = ServiceProvider::build(None).await.unwrap();
        lazy.dispose_async().await.unwrap();
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 0);

        let eager = ServiceProvider::build(Some(ServiceProviderOptions {
            initialization: InitializationMode::Eager,
            ..Default::default()
        }))
        .await
        .unwrap();
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
        assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);
        assert_eq!(CLEANUPS.load(Ordering::SeqCst), 0);
        eager.dispose_async().await.unwrap();
        assert_eq!(CLEANUPS.load(Ordering::SeqCst), 1);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);

        SHOULD_FAIL.store(true, Ordering::SeqCst);
        assert_eq!(
            graph,
            crate::graph::snapshot(&crate::graph::plan::CompiledApplication::load().graph)
                .to_string()
        );
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 1);
        match ServiceProvider::build(Some(ServiceProviderOptions {
            initialization: InitializationMode::Eager,
            ..Default::default()
        }))
        .await
        {
            Err(BuildError::Initialization {
                error,
                dispose_error,
            }) => {
                assert!(
                    error
                        .to_string()
                        .contains("graph snapshot initialization sentinel")
                );
                assert!(dispose_error.is_none());
            }
            Err(error) => panic!("expected initialization failure, got {error}"),
            Ok(_) => panic!("the failing factory must prevent successful eager build"),
        }
        assert_eq!(ATTEMPTS.load(Ordering::SeqCst), 2);
        assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);
        assert_eq!(CLEANUPS.load(Ordering::SeqCst), 1);
        assert_eq!(DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(
            graph,
            crate::graph::snapshot(&crate::graph::plan::CompiledApplication::load().graph)
                .to_string()
        );
    });
}
