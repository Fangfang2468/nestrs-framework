#![allow(dead_code)]
use nestrs::injectable;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static BUILDS: AtomicUsize = AtomicUsize::new(0);
static QUERIES: AtomicUsize = AtomicUsize::new(0);
struct Missing;

#[injectable]
struct Repository<T: Send + Sync + 'static> {
    #[cfg(feature = "invalid")]
    #[inject]
    missing: Missing,
    #[value(BUILDS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    marker: PhantomData<T>,
}

pub struct Runner<'a, T> {
    provider: &'a ServiceProvider,
    marker: PhantomData<T>,
}
impl<'a, T> Runner<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self {
            provider,
            marker: PhantomData,
        }
    }
}
impl<T: Send + Sync + 'static> Clone for Runner<'_, T> {
    fn clone(&self) -> Self {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let value = self
                    .provider
                    .get_required_service::<Repository<T>>()
                    .await
                    .expect("Clone must retain its closed service query");
                assert!(value.id < 2);
                QUERIES.fetch_add(1, Ordering::SeqCst);
            })
        });
        Self::new(self.provider)
    }
}

pub async fn build() -> ServiceProvider {
    ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    })
    .await
    .unwrap()
}

pub async fn finish(provider: ServiceProvider, builds: usize, queries: usize) {
    assert_eq!(
        BUILDS.load(Ordering::SeqCst),
        builds,
        "only actually compiled closed Clone routes become roots"
    );
    assert_eq!(QUERIES.load(Ordering::SeqCst), queries);
    provider.dispose_async().await.unwrap();
    println!("clone query contracts passed: builds={builds} queries={queries}");
}
