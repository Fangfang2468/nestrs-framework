//! All relevant concrete and dependency types remain private in the provider.

use contracts::{EntityReader, IdentityPort, RepositoryPort, UserEntity};
use fallback_provider as _;
use nestrs_core::ServiceProvider;
use primary_provider as _;

type Reader = dyn EntityReader<Entity = UserEntity> + Send + Sync;
type Repository = dyn RepositoryPort<UserEntity> + Send + Sync;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // The imports above are the only references to the implementation crates.
    // This application knows only their public contracts, not concrete types.
    let provider = ServiceProvider::build().await.unwrap();
    let repository = provider.get_required_service::<Repository>().await.unwrap();
    let reader = provider.get_required_service::<Reader>().await.unwrap();
    let identity = provider
        .get_required_service::<dyn IdentityPort>()
        .await
        .unwrap();
    assert_eq!(repository.count(), 23);
    assert_eq!(reader.entity_name(), "UserEntity");
    assert_eq!(repository.identity(), reader.identity());
    assert_eq!(identity.identity(), reader.identity());
    provider.dispose_async().await.unwrap();
    println!(
        "cross-crate generics: private closed blueprint, nested private trait, associated type and supertrait passed"
    );
}
