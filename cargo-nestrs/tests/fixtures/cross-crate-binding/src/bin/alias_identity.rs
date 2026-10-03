//! Same local type names in different crates and renamed public exports stay distinct.

use contracts::{CatalogPort as PublicCatalogPort, DeliveryPort};
use nestrs_core::{ServiceKey, ServiceProvider};
use primary_provider::Catalog as PublicCatalog;
use std::any::TypeId;

type Preferred = primary_provider::Service;
type Alternate = fallback_provider::Service;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_ne!(TypeId::of::<Preferred>(), TypeId::of::<Alternate>());
    let provider = ServiceProvider::build().await.unwrap();
    let catalog = provider
        .get_required_service::<PublicCatalog>()
        .await
        .unwrap();
    let port = provider
        .get_required_service::<dyn PublicCatalogPort>()
        .await
        .unwrap();
    let preferred = provider.get_required_service::<Preferred>().await.unwrap();
    let alternate = provider.get_required_service::<Alternate>().await.unwrap();
    let selected = provider
        .get_required_service::<dyn DeliveryPort>()
        .await
        .unwrap();
    let keyed_concrete = provider
        .get_required_keyed_service::<Preferred>(ServiceKey::Named("audit".into()))
        .await
        .unwrap();
    let keyed_interface = provider
        .get_required_keyed_service::<dyn DeliveryPort>(ServiceKey::Named("audit".into()))
        .await
        .unwrap();
    assert_eq!(port.identity(), catalog as *const PublicCatalog as usize);
    assert_eq!(selected.identity(), preferred as *const Preferred as usize);
    assert_eq!(
        keyed_interface.identity(),
        keyed_concrete as *const Preferred as usize
    );
    assert_ne!(selected.identity(), alternate as *const Alternate as usize);
    assert_ne!(preferred.id(), keyed_concrete.id());
    assert_eq!(alternate.source(), "fallback");
    assert_eq!(keyed_interface.source(), "audit");
    provider.dispose_async().await.unwrap();
    println!(
        "cross-crate aliases: reexports, distinct crate identities and keyed projection reuse passed"
    );
}
