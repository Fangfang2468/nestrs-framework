use nestrs_core::{
    __private::{BoundKeyPolicy, Provider, REFLECTED_PROVIDERS, TraitBinding},
    registration::service_type::ServiceType,
};
use nestrs_macro::bind;

trait Greeter: Send + Sync {}

trait HealthCheck: Send + Sync {}

struct GreeterService;

struct HealthCheckService;

#[bind]
impl Greeter for GreeterService {}

#[bind]
impl HealthCheck for HealthCheckService {}

#[test]
fn bind_collects_typed_bound_providers() {
    let bindings: Vec<_> = REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .filter_map(|provider| match provider {
            Provider::Bound(TraitBinding {
                trait_type,
                concrete_type,
                key_policy,
                ..
            }) => Some((trait_type, concrete_type, key_policy)),
            _ => None,
        })
        .collect();

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|(trait_type, concrete_type, key_policy)| {
        *concrete_type == ServiceType::create::<GreeterService>()
            && *trait_type == ServiceType::create::<dyn Greeter>()
            && *key_policy == BoundKeyPolicy::InheritRequestedKey
    }));
    assert!(bindings.iter().any(|(trait_type, concrete_type, key_policy)| {
        *concrete_type == ServiceType::create::<HealthCheckService>()
            && *trait_type == ServiceType::create::<dyn HealthCheck>()
            && *key_policy == BoundKeyPolicy::InheritRequestedKey
    }));
}
