use crate::registration::binding::TraitBinding;
use crate::service::{ServiceIdentifier, ServiceKey, ServiceType};
use nestrs::{bind, injectable};

trait Greeter: Send + Sync {}

trait HealthCheck: Send + Sync {}

#[injectable(key = "greeter")]
struct GreeterService;

#[injectable(key = 7)]
struct HealthCheckService;

#[bind]
impl Greeter for GreeterService {}

#[bind]
impl HealthCheck for HealthCheckService {}

#[test]
fn bind_collects_typed_trait_bindings() {
    let bindings: Vec<TraitBinding> = crate::registration::catalog::collect().bindings;

    assert_eq!(bindings.len(), 2);
    assert!(bindings.iter().any(|binding| {
        binding.concrete_type == ServiceType::create::<GreeterService>()
            && binding.trait_type == ServiceType::create::<dyn Greeter>()
    }));
    assert!(bindings.iter().any(|binding| {
        binding.concrete_type == ServiceType::create::<HealthCheckService>()
            && binding.trait_type == ServiceType::create::<dyn HealthCheck>()
    }));
}

#[test]
fn bind_inherits_concrete_keys_without_default_or_cross_key_fallback() {
    // 用真实图行为验证 key 继承，避免只断言一个不被图编译器读取的策略标签。
    let graph = crate::graph::GraphCompiler::compile_static().unwrap();
    for (interface, concrete, key) in [
        (
            ServiceType::create::<dyn Greeter>(),
            ServiceType::create::<GreeterService>(),
            ServiceKey::Named("greeter".to_owned()),
        ),
        (
            ServiceType::create::<dyn HealthCheck>(),
            ServiceType::create::<HealthCheckService>(),
            ServiceKey::Indexed(7),
        ),
    ] {
        let interface_route = &graph.routes[&ServiceIdentifier::new(Some(key.clone()), interface)];
        let concrete_route = &graph.routes[&ServiceIdentifier::new(Some(key.clone()), concrete)];
        assert_eq!(interface_route.provider, concrete_route.provider);
        assert_eq!(
            graph.nodes[interface_route.provider].identifier.service_key,
            Some(key)
        );
        assert!(interface_route.projection.is_some());
        assert!(
            !graph
                .routes
                .contains_key(&ServiceIdentifier::from(interface))
        );
    }
    assert!(!graph.routes.contains_key(&ServiceIdentifier::new(
        Some(ServiceKey::Indexed(7)),
        ServiceType::create::<dyn Greeter>(),
    )));
    assert!(!graph.routes.contains_key(&ServiceIdentifier::new(
        Some(ServiceKey::Named("greeter".to_owned())),
        ServiceType::create::<dyn HealthCheck>(),
    )));
}
