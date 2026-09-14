use nestrs_core::__private::{BoundKeyPolicy, REFLECTED_BINDINGS, ServiceType, TraitBinding};
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
fn bind_collects_typed_trait_bindings() {
    let bindings: Vec<TraitBinding> = REFLECTED_BINDINGS.iter().map(|binding| binding()).collect();

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|binding| {
        binding.concrete_type == ServiceType::create::<GreeterService>()
            && binding.trait_type == ServiceType::create::<dyn Greeter>()
            && binding.key_policy == BoundKeyPolicy::InheritRequestedKey
    }));
    assert!(bindings.iter().any(|binding| {
        binding.concrete_type == ServiceType::create::<HealthCheckService>()
            && binding.trait_type == ServiceType::create::<dyn HealthCheck>()
            && binding.key_policy == BoundKeyPolicy::InheritRequestedKey
    }));
}
