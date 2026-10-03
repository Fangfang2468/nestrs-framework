//! Only this executable contributes query roots; providers have no local demand.

use contracts::{CatalogPort, ConnectionPort, ConnectionView};
use nestrs_core::ServiceProvider;
use primary_provider::Catalog;
use upstream_consumer::Checkout;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(upstream_consumer::linked_provider_constructions(), 0);
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(primary_provider::total_constructions(), 0);
    let left = provider.create_scope();
    let right = provider.create_scope();
    let checkout = left
        .service_provider()
        .get_required_service::<Checkout>()
        .await
        .unwrap();
    let second = right
        .service_provider()
        .get_required_service::<Checkout>()
        .await
        .unwrap();
    let catalog = provider.get_required_service::<Catalog>().await.unwrap();
    let catalog_port = provider
        .get_required_service::<dyn CatalogPort>()
        .await
        .unwrap();
    let connection = provider
        .get_required_service::<dyn ConnectionPort>()
        .await
        .unwrap();
    let view = provider
        .get_required_service::<dyn ConnectionView>()
        .await
        .unwrap();
    let fallback = provider
        .get_required_service::<fallback_provider::Service>()
        .await
        .unwrap();
    assert_eq!(checkout.available(), 17);
    assert_eq!(
        checkout.catalog_identity(),
        catalog as *const Catalog as usize
    );
    assert_eq!(catalog_port.identity(), checkout.catalog_identity());
    assert_eq!(checkout.connection_identity(), connection.identity());
    assert_eq!(checkout.connection_id(), connection.connection_id());
    assert_eq!(view.view_identity(), connection.identity());
    assert_eq!(view.view_connection_id(), connection.connection_id());
    assert_eq!(checkout.connection_identity(), second.connection_identity());
    assert!(!std::ptr::eq(checkout, second));
    assert_eq!(checkout.delivery_source(), "primary");
    assert_eq!(fallback.id(), 99);
    assert_eq!(checkout.carrier(), "fallback-tracker");
    assert_eq!(checkout.tracking_identity(), second.tracking_identity());
    assert!(!checkout.has_fraud_plugin());
    left.dispose_async().await.unwrap();
    right.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(primary_provider::connection_cleanup_count(), 1);
    assert_eq!(primary_provider::connection_drop_count(), 1);
    println!(
        "cross-crate downstream demand: public class and private factory share singleton projections"
    );
}
