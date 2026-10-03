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
fn bind_freezes_typed_routes_without_additional_service_nodes() {
    let graph = &crate::graph::plan::load().graph;
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(
        graph
            .routes
            .values()
            .filter(|route| route.projection.is_some())
            .count(),
        2
    );
    assert_eq!(
        graph
            .routes
            .values()
            .filter(|route| route.projection.is_none())
            .count(),
        2
    );
}

#[test]
fn bind_inherits_concrete_keys_without_default_or_cross_key_fallback() {
    // 直接验证工具链产出的运行期计划；不在测试中再次执行另一份候选选择算法。
    let graph = &crate::graph::plan::load().graph;
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
