use nestrs::{factory, injectable};
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};

use std::{
    num::NonZeroUsize,
    sync::atomic::{AtomicUsize, Ordering},
};

static SHARED_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static SCOPED_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static TRANSIENT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CONSUMERS: AtomicUsize = AtomicUsize::new(0);
static GOOD_CLEANUPS: AtomicUsize = AtomicUsize::new(0);
struct Failed;
struct ScopedFailed;
struct TransientFailed;
struct Panicked;
#[derive(Debug)]
struct Failure(&'static str);
#[factory]
fn fails() -> Result<Failed, Failure> {
    SHARED_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    let failure = Failure("database unavailable");
    assert_eq!(failure.0, "database unavailable");
    Err(failure)
}
#[factory(lifetime = Scoped)]
async fn scoped_fails() -> Result<ScopedFailed, &'static str> {
    SCOPED_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    Err("scope unavailable")
}
#[factory(lifetime = Transient)]
fn transient_fails() -> Result<TransientFailed, &'static str> {
    TRANSIENT_ATTEMPTS.fetch_add(1, Ordering::SeqCst);
    Err("retry next occurrence")
}
#[factory]
fn panics() -> Panicked {
    panic!("factory panic sentinel")
}
#[injectable]
struct Consumer {
    #[inject]
    failed: Failed,
    #[value(CONSUMERS.fetch_add(1, Ordering::SeqCst))]
    value: usize,
}
async fn good_cleanup() {
    GOOD_CLEANUPS.fetch_add(1, Ordering::SeqCst);
}
#[injectable(cleanup = "good_cleanup")]
struct Good;

#[tokio::test]
async fn initialization_errors_are_cached_and_do_not_close_other_services() {
    let provider = ServiceProvider::build().await.unwrap();
    nestrs_core::get_required_service!(provider, Good)
        .await
        .unwrap();
    let first = nestrs_core::get_service!(provider, Failed)
        .await
        .err()
        .unwrap()
        .to_string();
    let second = nestrs_core::get_required_service!(provider, Failed)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(first, second);
    assert!(first.contains("database unavailable") && first.contains("runtime_failures.rs"));
    assert_eq!(SHARED_ATTEMPTS.load(Ordering::SeqCst), 1);
    assert!(
        nestrs_core::get_required_service!(provider, Consumer)
            .await
            .is_err()
    );
    assert_eq!(CONSUMERS.load(Ordering::SeqCst), 0);
    for _ in 0..2 {
        assert!(
            nestrs_core::get_required_service!(provider, TransientFailed)
                .await
                .is_err()
        );
    }
    assert_eq!(TRANSIENT_ATTEMPTS.load(Ordering::SeqCst), 2);
    let panic = nestrs_core::get_required_service!(provider, Panicked)
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(panic.contains("factory panic sentinel"));
    for _ in 0..2 {
        let scope = provider.create_scope();
        for _ in 0..2 {
            assert!(
                nestrs_core::get_required_service!(scope.service_provider(), ScopedFailed)
                    .await
                    .is_err()
            );
        }
        scope.dispose_async().await.unwrap();
    }
    assert_eq!(SCOPED_ATTEMPTS.load(Ordering::SeqCst), 2);
    assert_eq!(GOOD_CLEANUPS.load(Ordering::SeqCst), 0);
    nestrs_core::get_required_service!(provider, Good)
        .await
        .unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(GOOD_CLEANUPS.load(Ordering::SeqCst), 1);

    // Eager starts every singleton request before reporting failure, then cleans all successes.
    let eager = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        max_concurrent_activations: NonZeroUsize::new(4).unwrap(),
    })
    .await;
    assert!(eager.is_err());
    assert_eq!(GOOD_CLEANUPS.load(Ordering::SeqCst), 2);
}
