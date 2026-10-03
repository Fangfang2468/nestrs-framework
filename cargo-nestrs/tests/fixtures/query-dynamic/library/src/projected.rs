//! The parent trait parameter is an associated projection, not a source-name alias.
use super::{Query, Rejected, Runner, invalid, query};
use nestrs_core::ServiceProvider;

pub trait Map {
    type Target: Send + Sync + 'static;
}

pub trait Run<T: Send + Sync + 'static> {
    fn projected<'a>(&self, provider: &'a ServiceProvider) -> Query<'a>;
}
pub trait Child<M: Map>: Run<M::Target> {}

impl<T: Send + Sync + 'static> Run<T> for Runner<T> {
    fn projected<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        query::<T>(provider)
    }
}
impl<M: Map> Child<M> for Runner<M::Target> {}

pub fn erase<M: Map + 'static>() -> Box<dyn Child<M>> {
    Box::new(Runner::<M::Target>::new())
}

impl<T: Send + Sync + 'static> Run<T> for Rejected<T> {
    fn projected<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<T>(provider)
    }
}
impl<M: Map> Child<M> for Rejected<M::Target> {}

pub fn erase_rejected<M: Map + 'static>() -> Box<dyn Child<M>> {
    Box::new(Rejected::<M::Target>::new())
}

pub trait Nested<M: Map>: Child<M::Target>
where
    M::Target: Map,
{
}
impl<M: Map> Nested<M> for Runner<<M::Target as Map>::Target> where M::Target: Map {}

pub fn erase_nested<M: Map + 'static>() -> Box<dyn Nested<M>>
where
    M::Target: Map,
{
    Box::new(Runner::<<M::Target as Map>::Target>::new())
}

pub trait DefaultRun<T: Send + Sync + 'static> {
    fn projected_default<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        query::<T>(provider)
    }
}
pub trait DefaultChild<M: Map>: DefaultRun<M::Target> {}
impl<T: Send + Sync + 'static> DefaultRun<T> for Runner<T> {}
impl<M: Map> DefaultChild<M> for Runner<M::Target> {}

pub fn erase_default<M: Map + 'static>() -> Box<dyn DefaultChild<M>> {
    Box::new(Runner::<M::Target>::new())
}

pub trait InvalidRun<T: Send + Sync + 'static> {
    fn projected_invalid<'a>(&self, provider: &'a ServiceProvider) -> Query<'a> {
        invalid::<T>(provider)
    }
}
pub trait InvalidChild<M: Map>: InvalidRun<M::Target> {}
impl<T: Send + Sync + 'static> InvalidRun<T> for Runner<T> {}
impl<M: Map> InvalidChild<M> for Runner<M::Target> {}

pub fn erase_invalid<M: Map + 'static>() -> Box<dyn InvalidChild<M>> {
    Box::new(Runner::<M::Target>::new())
}
