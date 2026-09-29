//! The consumer knows only contracts; the executable links both provider crates.

use contracts::{DeliveryPort, FraudPlugin};
use nestrs_core::{
    ServiceKey, ServiceProvider, get_required_keyed_service, get_required_service, get_service,
};
use sibling_consumer::Dispatch;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    let dispatch = get_required_service!(scope.service_provider(), Dispatch)
        .await
        .unwrap();
    let primary = get_required_service!(provider, primary_provider::Service)
        .await
        .unwrap();
    let fallback = get_required_service!(provider, fallback_provider::Service)
        .await
        .unwrap();
    let audit = get_required_keyed_service!(
        provider,
        dyn DeliveryPort,
        ServiceKey::Named("audit".into())
    )
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
        get_service!(provider, dyn FraudPlugin)
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
