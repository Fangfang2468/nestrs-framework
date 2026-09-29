//! Repeated demands in two libraries and this binary must not duplicate pairs.

use contracts::{CatalogPort, ConnectionPort, DeliveryPort};
use nestrs_core::{
    __private::{REFLECTED_AUTOMATIC_BINDINGS, ServiceType},
    ServiceProvider, get_required_service,
};
use sibling_consumer::Dispatch;
use upstream_consumer::Checkout;

fn has_pair<Concrete: Send + Sync + 'static, Interface: ?Sized + Send + Sync + 'static>() -> bool {
    REFLECTED_AUTOMATIC_BINDINGS
        .iter()
        .map(|declare| declare())
        .any(|binding| {
            binding.concrete_type == ServiceType::create::<Concrete>()
                && binding.trait_type == ServiceType::create::<Interface>()
        })
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(upstream_consumer::linked_provider_constructions(), 0);
    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    let checkout = get_required_service!(scope.service_provider(), Checkout)
        .await
        .unwrap();
    let dispatch = get_required_service!(scope.service_provider(), Dispatch)
        .await
        .unwrap();
    let catalog = get_required_service!(provider, dyn CatalogPort)
        .await
        .unwrap();
    let preferred = get_required_service!(provider, dyn DeliveryPort)
        .await
        .unwrap();
    let private = get_required_service!(provider, dyn ConnectionPort)
        .await
        .unwrap();
    let fallback = get_required_service!(provider, fallback_provider::Service)
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
    assert!(
        REFLECTED_AUTOMATIC_BINDINGS
            .iter()
            .map(|declare| declare())
            .any(|binding| binding.trait_type == ServiceType::create::<dyn ConnectionPort>()),
    );
    // Multiple automatic declaration callbacks may advertise the same pair.
    // Successful graph construction and these shared identities prove logical
    // deduplication; the physical linkme directory need not contain one record.
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(primary_provider::connection_cleanup_count(), 1);
    assert_eq!(primary_provider::connection_drop_count(), 1);
    println!("cross-crate transitive reuse: repeated automatic pairs share one logical route");
}
