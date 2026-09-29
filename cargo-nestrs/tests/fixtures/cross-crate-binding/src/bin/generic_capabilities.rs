//! All relevant concrete and dependency types remain private in the provider.

use contracts::{EntityReader, IdentityPort, RepositoryPort, UserEntity};
use fallback_provider as _;
use nestrs_core::{ServiceProvider, get_required_service};
use primary_provider as _;

type Reader = dyn EntityReader<Entity = UserEntity> + Send + Sync;
type Repository = dyn RepositoryPort<UserEntity> + Send + Sync;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    // The imports above are the only references to the implementation crates.
    // This application knows only their public contracts, not concrete types.
    let provider = ServiceProvider::build().await.unwrap();
    let repository = get_required_service!(provider, Repository).await.unwrap();
    let reader = get_required_service!(provider, Reader).await.unwrap();
    let identity = get_required_service!(provider, dyn IdentityPort)
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
