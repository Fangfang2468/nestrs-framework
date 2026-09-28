//! The definition module is private; only its public reexports are usable downstream.
mod implementation {
    use nestrs::injectable;
    use std::marker::PhantomData;
    #[injectable]
    pub struct Cache<T> {
        marker: PhantomData<T>,
    }
    #[injectable]
    pub struct InternalRepository<T> {
        #[inject]
        cache: Cache<T>,
    }
    pub trait RepositoryPort: Send + Sync {
        fn count(&self) -> usize;
    }
    impl<T: Send + Sync + 'static> RepositoryPort for InternalRepository<T> {
        fn count(&self) -> usize {
            let _ = &self.cache;
            1
        }
    }
    #[injectable(key = "replica")]
    pub struct Named<T> {
        marker: PhantomData<T>,
    }
    #[injectable(key = 7)]
    pub struct Indexed<T> {
        marker: PhantomData<T>,
    }
    pub struct KnownUser;
    pub trait KnownPort: Send + Sync {
        fn value(&self) -> usize;
    }
    impl KnownPort for InternalRepository<KnownUser> {
        fn value(&self) -> usize {
            23
        }
    }

    /// This compiled but never executed query causes an upstream automatic
    /// binding, which a downstream query must reuse rather than emit again.
    pub async fn known_request(provider: &nestrs_core::ServiceProvider) -> usize {
        let _ = nestrs_core::get_required_service!(provider, InternalRepository<KnownUser>)
            .await
            .unwrap();
        nestrs_core::get_required_service!(provider, dyn KnownPort)
            .await
            .unwrap()
            .value()
    }
}
pub use implementation::{
    Cache, Indexed, InternalRepository as Repository, KnownPort, KnownUser, Named, RepositoryPort,
    known_request,
};
