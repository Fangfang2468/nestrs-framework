//! 只验证门面签名能够被下游 crate 使用；不调用尚未实现的 runtime 占位方法。

use std::error::Error;

use nestrs_core::{
    BuildError, ResolveError, ServiceKey, ServiceProvider, ServiceScope, ShutdownError,
};

struct Concrete;

trait Port: Send + Sync {}

fn assert_error<E: Error>() {}

fn accepts_public_service_key(_: ServiceKey) {}

fn provider_api(provider: &ServiceProvider) {
    std::mem::drop(ServiceProvider::build());
    std::mem::drop(provider.get::<Concrete>());
    std::mem::drop(provider.try_get::<Concrete>());
    std::mem::drop(provider.get_keyed::<Concrete>(ServiceKey::Named("primary".to_owned())));
    std::mem::drop(provider.try_get_keyed::<Concrete>(ServiceKey::Indexed(1)));
    std::mem::drop(provider.get::<dyn Port>());
    let _ = provider.create_scope();
}

fn scope_api(scope: &ServiceScope<'_>) {
    std::mem::drop(scope.get::<Concrete>());
    std::mem::drop(scope.try_get::<Concrete>());
    std::mem::drop(scope.get_keyed::<Concrete>(ServiceKey::Named("primary".to_owned())));
    std::mem::drop(scope.try_get_keyed::<Concrete>(ServiceKey::Indexed(1)));
    std::mem::drop(scope.get::<dyn Port>());
}

fn provider_shutdown_api(provider: ServiceProvider) {
    std::mem::drop(provider.shutdown());
}

fn scope_shutdown_api(scope: ServiceScope<'_>) {
    std::mem::drop(scope.shutdown());
}

#[test]
fn public_facade_types_and_signatures_are_available() {
    let _ = provider_api;
    let _ = scope_api;
    let _ = provider_shutdown_api;
    let _ = scope_shutdown_api;

    assert_error::<BuildError>();
    assert_error::<ResolveError>();
    assert_error::<ShutdownError>();

    accepts_public_service_key(nestrs_core::__private::ServiceKey::Named(
        "macro-abi".to_owned(),
    ));
}
