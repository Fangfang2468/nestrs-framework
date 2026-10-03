//! Closed generic impl roots, associated types, and legal private scopes.

use nestrs::injectable;
use nestrs_core::ServiceProvider;
use std::marker::PhantomData;

#[path = "semantic_edges/private_module.rs"]
mod private_module;

struct User;

trait CachePort: Send + Sync {
    type Entity;
    fn identity(&self) -> usize;
}

#[injectable]
struct Cache<T> {
    marker: PhantomData<T>,
    #[value(7)]
    _allocation: usize,
}

impl<T: Send + Sync> CachePort for Cache<T> {
    type Entity = T;

    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

trait RepositoryPort: Send + Sync {
    fn cache_identity(&self) -> usize;
}

#[injectable]
struct Repository<T> {
    #[inject]
    cache: Cache<T>,
}

// No concrete query or dependency mentions Repository<User>. The explicitly
// closed ordinary impl is a finite seed when RepositoryPort is requested.
impl RepositoryPort for Repository<User> {
    fn cache_identity(&self) -> usize {
        (&*self.cache) as *const Cache<User> as usize
    }
}

// A legal, unrelated impl can live inside a function. It does not change the
// source module in which the provider/interface pair is safely expressible.
#[allow(dead_code, non_local_definitions)]
fn unrelated_impl_scope() {
    impl std::fmt::Debug for Repository<User> {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("Repository<User>")
        }
    }
}

type RequestedCache = dyn CachePort<Entity = User> + Send + Sync;

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let repository = provider
        .get_required_service::<dyn RepositoryPort>()
        .await
        .unwrap();
    let cache = provider
        .get_required_service::<RequestedCache>()
        .await
        .unwrap();
    assert_eq!(repository.cache_identity(), cache.identity());
    private_module::assert_private_projection(&provider).await;
    provider.dispose_async().await.unwrap();
    println!("auto-binding semantic edges: private scope/associated types/generic chain passed");
}
