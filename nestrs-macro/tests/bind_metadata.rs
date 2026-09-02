use nestrs_core::{__private::REFLECT_METADATA_BIND, registration::service_type::ServiceType};
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
fn bind_collects_interface_metadata() {
    let bindings: Vec<_> = REFLECT_METADATA_BIND
        .iter()
        .map(|metadata| metadata())
        .collect();

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|binding| {
        binding.service_type == ServiceType::create::<GreeterService>()
            && binding.trait_type == ServiceType::create::<dyn Greeter>()
    }));
    assert!(bindings.iter().any(|binding| {
        binding.service_type == ServiceType::create::<HealthCheckService>()
            && binding.trait_type == ServiceType::create::<dyn HealthCheck>()
    }));
}
