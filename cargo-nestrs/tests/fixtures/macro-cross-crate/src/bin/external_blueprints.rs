//! These concrete generic roots first become known in this binary. Their
//! provider/dependency descriptors live in the separately compiled library.
use nestrs_core::{ServiceKey, ServiceProvider};
use nestrs_macro_cross_crate::{
    Cache, Indexed, Named, NestedWrapper, PrivateArgument, Repository, RepositoryPort, Wrapper,
};

struct User;
type UserRepository = Repository<User>;
type Replica = Named<User>;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let repository = provider
        .get_required_service::<UserRepository>()
        .await
        .unwrap();
    let port = provider
        .get_required_service::<dyn RepositoryPort<User>>()
        .await
        .unwrap();
    assert_eq!(repository.count(), 1);
    assert_eq!(port.count(), 1);
    assert!(std::ptr::addr_eq(
        repository as *const UserRepository,
        port as *const dyn RepositoryPort<User>,
    ));
    // The Cache dependency was obtained from encoded MIR and expanded through
    // the same finite queue as local generic provider declarations.
    let _ = provider
        .get_required_service::<Cache<User>>()
        .await
        .unwrap();
    let _ = provider
        .get_required_keyed_service::<Replica>(ServiceKey::Named("replica".into()))
        .await
        .unwrap();
    let _ = provider
        .get_required_keyed_service::<Indexed<User>>(ServiceKey::Indexed(7))
        .await
        .unwrap();
    // These T requests only become closed in this downstream crate. No direct
    // query for Cache<u16> or Repository<u16> may seed the provider directory.
    let wrapped = provider
        .get_required_service::<Wrapper<Repository<u16>>>()
        .await
        .unwrap();
    assert_eq!(wrapped.service().count(), 1);
    let nested = provider
        .get_required_service::<NestedWrapper<Repository<u16>>>()
        .await
        .unwrap();
    assert!(std::ptr::eq(wrapped.service(), nested.service()));
    type PortAlias = dyn RepositoryPort<User>;
    let trait_wrapper = provider
        .get_required_service::<Wrapper<PortAlias>>()
        .await
        .unwrap();
    assert_eq!(trait_wrapper.service().count(), 1);
    // Both nested service types are private upstream. The leaf travels through
    // Wrapper<T>, whose ordinary generic probe cannot know T's blueprint.
    assert!(
        provider
            .get_required_service::<PrivateArgument<u8>>()
            .await
            .unwrap()
            .resolved()
    );
    provider.dispose_async().await.unwrap();
    println!("external blueprints: chain, alias, shared trait identity and all keys passed");
}
