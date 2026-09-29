//! Same local type names in different crates and renamed public exports stay distinct.

use contracts::{CatalogPort as PublicCatalogPort, DeliveryPort};
use nestrs_core::{ServiceKey, ServiceProvider, get_required_keyed_service, get_required_service};
use primary_provider::Catalog as PublicCatalog;
use std::any::TypeId;

type Preferred = primary_provider::Service;
type Alternate = fallback_provider::Service;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_ne!(TypeId::of::<Preferred>(), TypeId::of::<Alternate>());
    let provider = ServiceProvider::build().await.unwrap();
    let catalog = get_required_service!(provider, PublicCatalog)
        .await
        .unwrap();
    let port = get_required_service!(provider, dyn PublicCatalogPort)
        .await
        .unwrap();
    let preferred = get_required_service!(provider, Preferred).await.unwrap();
    let alternate = get_required_service!(provider, Alternate).await.unwrap();
    let selected = get_required_service!(provider, dyn DeliveryPort)
        .await
        .unwrap();
    let keyed_concrete =
        get_required_keyed_service!(provider, Preferred, ServiceKey::Named("audit".into()))
            .await
            .unwrap();
    let keyed_interface = get_required_keyed_service!(
        provider,
        dyn DeliveryPort,
        ServiceKey::Named("audit".into())
    )
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
