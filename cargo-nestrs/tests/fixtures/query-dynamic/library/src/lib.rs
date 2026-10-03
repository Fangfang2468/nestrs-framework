//! Ordinary business objects are deliberately not DI providers.
#![allow(dead_code)]
pub mod projected;
use nestrs::injectable;
use nestrs_core::ServiceProvider;
use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static BUILDS: AtomicUsize = AtomicUsize::new(0);
static QUERIES: AtomicUsize = AtomicUsize::new(0);
#[injectable]
struct Repository<T: Send + Sync + 'static> {
    #[value(BUILDS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    marker: PhantomData<T>,
}
struct Unregistered;
#[injectable]
struct InvalidRepository<T: Send + Sync + 'static> {
    #[inject]
    missing: Unregistered,
    marker: PhantomData<T>,
}
pub fn builds() -> usize {
    BUILDS.load(Ordering::SeqCst)
}
pub fn queries() -> usize {
    QUERIES.load(Ordering::SeqCst)
}
pub type Query<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> Query<'_> {
    Box::pin(async move {
        let _ = provider
            .get_required_service::<Repository<T>>()
            .await
            .unwrap();
        QUERIES.fetch_add(1, Ordering::SeqCst);
    })
}
fn invalid<T: Send + Sync + 'static>(provider: &ServiceProvider) -> Query<'_> {
    Box::pin(async move {
        let _ = provider
            .get_required_service::<InvalidRepository<T>>()
            .await
            .unwrap();
    })
}
pub trait Run {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> Query<'a>;
    fn unused<'a>(&self, provider: &'a ServiceProvider) -> Query<'a>;
}
pub struct Runner<T>(PhantomData<T>);
impl<T> Runner<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T: Send + Sync + 'static> Run for Runner<T> {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        query::<T>(provider)
    }
    fn unused<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<T>(provider)
    }
}
pub trait RunMut {
    fn run_mut<'a>(&mut self, provider: &'a ServiceProvider) -> Query<'a>;
}
impl<T: Send + Sync + 'static> RunMut for Runner<T> {
    fn run_mut<'a>(&mut self, provider: &'a ServiceProvider) -> Query<'a> {
        query::<T>(provider)
    }
}
pub fn erase<T: Send + Sync + 'static>(runner: Runner<T>) -> Box<dyn Run> {
    Box::new(runner)
}
pub fn invoke<'a, T: Run + ?Sized>(runner: &T, provider: &'a ServiceProvider) -> Query<'a> {
    runner.run(provider)
}
pub fn dead<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    if false {
        drop(erase(Runner::<T>::new()).run(provider));
    }
}
pub trait Base {
    type Tag: Send + Sync + 'static;
    fn execute<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        query::<Self::Tag>(provider)
    }
}
pub trait Child: Base {}
impl<T: Send + Sync + 'static> Base for Runner<T> {
    type Tag = T;
}
impl<T: Send + Sync + 'static> Child for Runner<T> {}
pub fn erase_child<T: Send + Sync + 'static>() -> Arc<dyn Child<Tag = T> + Send + Sync> {
    Arc::new(Runner::<T>::new())
}

pub trait Dormant {
    fn invalid<'a>(&self, provider: &'a ServiceProvider) -> Query<'a>;
}
impl<T: Send + Sync + 'static> Dormant for Runner<T> {
    fn invalid<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<T>(provider)
    }
}
pub trait InvalidDefault {
    type Tag: Send + Sync + 'static;
    fn invalid_default<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<Self::Tag>(provider)
    }
}
impl<T: Send + Sync + 'static> InvalidDefault for Runner<T> {
    type Tag = T;
}

// Equal principal traits can still have incompatible object shapes.
pub trait Select {
    type Tag: Send + Sync + 'static;
    fn select<'a>(&self, provider: &'a ServiceProvider) -> Query<'a>;
}
pub struct Selected<T>(PhantomData<T>);
pub struct Rejected<T>(PhantomData<T>);
impl<T> Selected<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T> Rejected<T> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T: Send + Sync + 'static> Select for Selected<T> {
    type Tag = T;
    fn select<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        query::<T>(provider)
    }
}
impl<T: Send + Sync + 'static> Select for Rejected<T> {
    type Tag = T;
    fn select<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<T>(provider)
    }
}

pub trait InvalidRun {
    fn fail<'a>(&self, provider: &'a ServiceProvider) -> Query<'a>;
}
impl<T: Send + Sync + 'static> InvalidRun for Runner<T> {
    fn fail<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<T>(provider)
    }
}
