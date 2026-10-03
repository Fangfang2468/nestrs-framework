use crate as nestrs_core;
use nestrs_core::{
    BuildError, DisposeError, InitializationMode, ResolveError, ServiceKey, ServiceProvider,
    ServiceProviderOptions, ServiceScope,
};
use std::{error::Error, num::NonZeroUsize};

#[derive(Debug)]
struct Concrete;
trait Port: Send + Sync {}
fn assert_error<E: Error>() {}

fn provider_api(provider: &ServiceProvider) {
    std::mem::drop(ServiceProvider::build());
    std::mem::drop(ServiceProvider::build_with_options(
        ServiceProviderOptions {
            initialization: InitializationMode::Eager,
            max_concurrent_activations: NonZeroUsize::new(4).unwrap(),
        },
    ));
    std::mem::drop(provider.get_required_service::<Concrete>());
    std::mem::drop(provider.get_service::<Concrete>());
    std::mem::drop(
        provider.get_required_keyed_service::<Concrete>(ServiceKey::Named("primary".to_owned())),
    );
    std::mem::drop(provider.get_keyed_service::<Concrete>(ServiceKey::Indexed(1)));
    std::mem::drop(provider.get_required_service::<dyn Port>());
    let _ = provider.create_scope();
}
async fn chained_scope_api(scope: &ServiceScope<'_>) {
    let value = scope
        .service_provider()
        .get_required_service::<Concrete>()
        .await
        .unwrap();
    let _ = std::ptr::from_ref(value);
    std::mem::drop(scope.warm_up());
}
fn provider_disposal(provider: ServiceProvider) {
    std::mem::drop(provider.dispose_async());
}
fn scope_disposal(scope: ServiceScope<'_>) {
    std::mem::drop(scope.dispose_async());
}

#[test]
fn public_facade_types_and_signatures_are_available() {
    let _ = (
        provider_api,
        chained_scope_api,
        provider_disposal,
        scope_disposal,
    );
    assert_error::<BuildError>();
    assert_error::<ResolveError>();
    assert_error::<DisposeError>();
    assert_eq!(
        ServiceProviderOptions::default().initialization,
        InitializationMode::Lazy
    );
    assert_eq!(
        ServiceProviderOptions::default()
            .max_concurrent_activations
            .get(),
        32
    );
}

#[tokio::test]
async fn empty_provider_and_scope_complete_full_lifecycle() {
    let provider = ServiceProvider::build().await.unwrap();
    assert!(provider.get_service::<Concrete>().await.unwrap().is_none());
    assert!(
        provider
            .get_required_service::<Concrete>()
            .await
            .unwrap_err()
            .to_string()
            .contains("未注册")
    );
    let scope = provider.create_scope();
    scope.warm_up().await.unwrap();
    assert!(
        scope
            .service_provider()
            .get_service::<dyn Port>()
            .await
            .unwrap()
            .is_none()
    );
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}

#[test]
fn build_reports_missing_runtime_without_creating_one() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let mut build = std::pin::pin!(ServiceProvider::build());
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(
        build.as_mut().poll(&mut context),
        Poll::Ready(Err(BuildError::RuntimeUnavailable))
    ));
}
