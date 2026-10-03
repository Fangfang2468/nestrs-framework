use nestrs::{factory, injectable};
use nestrs_core::{Injection, ServiceProvider};

use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Semaphore;

static STARTED: Semaphore = Semaphore::const_new(0);
static RELEASE: Semaphore = Semaphore::const_new(0);
static CLEANUP_STARTED: Semaphore = Semaphore::const_new(0);
static CLEANUP_RELEASE: Semaphore = Semaphore::const_new(0);
static DEPENDENCY_CLOSED: Semaphore = Semaphore::const_new(0);
static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static DROPS: AtomicUsize = AtomicUsize::new(0);
static ORDER: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
static ESCAPED: Mutex<Option<Injection<Dependency>>> = Mutex::new(None);
struct Dependency {
    value: usize,
}
impl Drop for Dependency {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}
async fn dependency_cleanup() {
    ORDER.lock().unwrap().push("dependency");
    DEPENDENCY_CLOSED.add_permits(1);
}
#[factory(cleanup = "dependency_cleanup")]
async fn dependency() -> Dependency {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst);
    STARTED.add_permits(1);
    RELEASE.acquire().await.unwrap().forget();
    Dependency { value: 42 }
}
async fn consumer_cleanup() {
    ORDER.lock().unwrap().push("consumer-start");
    CLEANUP_STARTED.add_permits(1);
    CLEANUP_RELEASE.acquire().await.unwrap().forget();
    ORDER.lock().unwrap().push("consumer-end");
}
#[injectable(cleanup = "consumer_cleanup")]
struct Consumer {
    #[inject]
    dependency: Option<Dependency>,
}
impl Drop for Consumer {
    fn drop(&mut self) {
        *ESCAPED.lock().unwrap() = self.dependency.take();
    }
}
struct FactoryConsumer(usize);
#[factory]
async fn factory_consumer(dependency: Dependency) -> FactoryConsumer {
    tokio::task::yield_now().await;
    FactoryConsumer(dependency.value)
}
async fn panicking_cleanup() {
    panic!("cleanup panic sentinel");
}
#[injectable(cleanup = "panicking_cleanup")]
struct BadCleanup;

#[tokio::test]
async fn cancellation_keeps_accepted_work_and_close_running() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let mut query = Box::pin(provider.get_required_service::<Consumer>());
    tokio::select! {
        _ = STARTED.acquire() => {},
        _ = &mut query => panic!("factory should be waiting"),
    }
    drop(query);
    RELEASE.add_permits(1);
    assert_eq!(
        provider
            .get_required_service::<FactoryConsumer>()
            .await
            .unwrap()
            .0,
        42
    );
    provider.get_required_service::<Consumer>().await.unwrap();
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    let mut disposal = Box::pin(provider.dispose_async());
    tokio::select! {
        _ = CLEANUP_STARTED.acquire() => {},
        _ = &mut disposal => panic!("cleanup should be waiting"),
    }
    drop(disposal);
    assert_eq!(*ORDER.lock().unwrap(), ["consumer-start"]);
    CLEANUP_RELEASE.add_permits(1);
    DEPENDENCY_CLOSED.acquire().await.unwrap().forget();
    // Give the cleanup worker completion back to the coordinator.
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        *ORDER.lock().unwrap(),
        ["consumer-start", "consumer-end", "dependency"]
    );
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    let token = ESCAPED.lock().unwrap().take().unwrap();
    assert_eq!(token.value, 42);
    drop(token);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);

    // Ordinary Drop only sends Close, yet an active runtime completes cleanup.
    RELEASE.add_permits(1);
    let implicit = ServiceProvider::build(None).await.unwrap();
    implicit.get_required_service::<Dependency>().await.unwrap();
    drop(implicit);
    DEPENDENCY_CLOSED.acquire().await.unwrap().forget();

    RELEASE.add_permits(1);
    let panics = ServiceProvider::build(None).await.unwrap();
    panics.get_required_service::<Dependency>().await.unwrap();
    panics.get_required_service::<BadCleanup>().await.unwrap();
    let error = panics.dispose_async().await.unwrap_err();
    assert!(error.to_string().contains("cleanup panic sentinel"));
    assert_eq!(error.failures().len(), 1);
    assert_eq!(DROPS.load(Ordering::SeqCst), 3);
}
