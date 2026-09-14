use nestrs_core::{
    ServiceProvider,
    scope::{ScopeEnd, ScopeLayer, ScopeProvider},
};
use nestrs_macro::injectable;

#[injectable]
struct SharedSingleton;

#[injectable(lifetime = Transient)]
struct PerConsumerTransient;

#[injectable]
struct AppRoot {
    #[inject]
    singleton: SharedSingleton,
    #[inject]
    transient: PerConsumerTransient,
}

#[injectable(lifetime = Scoped)]
struct RequestService {
    #[inject]
    singleton: SharedSingleton,
}

#[injectable(lifetime = Scoped)]
struct RequestRoot {
    #[inject]
    singleton: SharedSingleton,
    #[inject]
    request: RequestService,
    #[inject]
    transient: PerConsumerTransient,
}

#[test]
fn root_provider_exposes_only_committed_persistent_services() {
    let provider = ServiceProvider::<AppRoot>::build().expect("root provider should build");

    assert!(std::ptr::eq(
        provider.root(),
        provider
            .get::<AppRoot>()
            .expect("the committed root should be visible"),
    ));
    assert!(provider.get::<SharedSingleton>().is_some());
    assert!(provider.get::<PerConsumerTransient>().is_none());
}

#[test]
fn scope_reads_current_scoped_and_parent_singleton_services() {
    type RequestScopes = ScopeLayer<RequestRoot, ScopeEnd>;

    let provider = ScopeProvider::<AppRoot, RequestScopes>::build()
        .expect("scope provider should build its singleton graph");
    let scope = provider
        .create_scope()
        .expect("request scope should build its scoped graph");

    assert!(std::ptr::eq(
        scope.root(),
        scope
            .get::<RequestRoot>()
            .expect("the current scope root should be visible"),
    ));
    assert!(scope.get::<RequestService>().is_some());
    assert!(scope.get::<SharedSingleton>().is_some());
    assert!(scope.get::<PerConsumerTransient>().is_none());
}
