//! Repeated demands in two libraries and this binary must not duplicate pairs.

use contracts::{CatalogPort, ConnectionPort, DeliveryPort};
use nestrs_core::ServiceProvider;
use sibling_consumer::Dispatch;
use upstream_consumer::Checkout;

#[path = "../../../../support/compiler_bindings.rs"]
mod compiler_bindings;

fn has_pair<Concrete: Send + Sync + 'static, Interface: ?Sized + Send + Sync + 'static>() -> bool {
    compiler_bindings::pair_count::<Concrete, Interface>() > 0
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(upstream_consumer::linked_provider_constructions(), 0);
    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    let checkout = scope
        .service_provider()
        .get_required_service::<Checkout>()
        .await
        .unwrap();
    let dispatch = scope
        .service_provider()
        .get_required_service::<Dispatch>()
        .await
        .unwrap();
    let catalog = provider
        .get_required_service::<dyn CatalogPort>()
        .await
        .unwrap();
    let preferred = provider
        .get_required_service::<dyn DeliveryPort>()
        .await
        .unwrap();
    let private = provider
        .get_required_service::<dyn ConnectionPort>()
        .await
        .unwrap();
    let fallback = provider
        .get_required_service::<fallback_provider::Service>()
        .await
        .unwrap();
    assert_eq!(fallback.source(), "fallback");
    assert_eq!(checkout.catalog_identity(), dispatch.catalog_identity());
    assert_eq!(catalog.identity(), checkout.catalog_identity());
    assert_eq!(preferred.identity(), checkout.delivery_identity());
    assert_eq!(preferred.identity(), dispatch.preferred_identity());
    assert_eq!(private.identity(), checkout.connection_identity());
    assert!(has_pair::<primary_provider::Catalog, dyn CatalogPort>());
    assert!(has_pair::<primary_provider::Service, dyn DeliveryPort>());
    assert!(has_pair::<fallback_provider::Service, dyn DeliveryPort>());
    assert!(compiler_bindings::count::<dyn ConnectionPort>() > 0);
    // Successful graph construction and shared identities prove that repeated
    // demands reuse one logical route; compiler records verify exact pairs.
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(primary_provider::connection_cleanup_count(), 1);
    assert_eq!(primary_provider::connection_drop_count(), 1);
    println!("cross-crate transitive reuse: repeated automatic pairs share one logical route");
}
