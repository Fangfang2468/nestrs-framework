//! The consumer knows only contracts; the executable links both provider crates.

use contracts::{DeliveryPort, FraudPlugin};
use nestrs_core::{ServiceKey, ServiceProvider};
use sibling_consumer::Dispatch;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    let dispatch = scope
        .service_provider()
        .get_required_service::<Dispatch>()
        .await
        .unwrap();
    let primary = provider
        .get_required_service::<primary_provider::Service>()
        .await
        .unwrap();
    let fallback = provider
        .get_required_service::<fallback_provider::Service>()
        .await
        .unwrap();
    let audit = provider
        .get_required_keyed_service::<dyn DeliveryPort>(ServiceKey::Named("audit".into()))
        .await
        .unwrap();
    assert_eq!(dispatch.preferred_source(), "primary");
    assert_eq!(dispatch.preferred_identity(), primary as *const _ as usize);
    assert_eq!(
        dispatch.optional_identity(),
        Some(dispatch.preferred_identity())
    );
    assert_eq!(dispatch.audit_source(), "audit");
    assert_eq!(dispatch.audit_identity(), audit.identity());
    assert_ne!(dispatch.preferred_identity(), audit.identity());
    assert_eq!(fallback.source(), "fallback");
    assert_eq!(fallback.id(), 99);
    assert!(!dispatch.has_fraud_plugin());
    assert!(
        provider
            .get_service::<dyn FraudPlugin>()
            .await
            .unwrap()
            .is_none()
    );
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    println!(
        "cross-crate siblings: primary, exact key and present/absent optional injection passed"
    );
}
