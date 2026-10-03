use crate::service::{ServiceIdentifier, ServiceKey, ServiceType};
use nestrs::{factory, injectable};
use nestrs_core::ServiceProvider;

use std::marker::PhantomData;

#[injectable(key = "root")]
struct Repository<T> {
    marker: PhantomData<T>,
}

struct User;
struct Order;
struct Missing;
struct FactoryOnly<T> {
    marker: PhantomData<T>,
}
trait Port: Send + Sync {}

type UserRepository = Repository<User>;
type PortAlias = dyn Port;

#[factory]
fn factory_only() -> FactoryOnly<User> {
    FactoryOnly {
        marker: PhantomData,
    }
}

// A compiled query site contributes a static root independently of function execution.
#[allow(dead_code)]
fn never_executed_queries(provider: &ServiceProvider) {
    drop(provider.get_service::<UserRepository>());
    drop(provider.get_required_service::<Repository<Order>>());
    if false {
        drop(provider.get_service::<Repository<User>>());
    }
    drop(provider.get_service::<FactoryOnly<User>>());
    drop(provider.get_service::<PortAlias>());
    drop(provider.get_service::<Missing>());
    #[cfg(any())]
    drop(provider.get_service::<ThisTypeDoesNotExist>());
}

#[test]
fn reflect_plan_closes_compiled_queries_without_registering_absent_fallbacks() {
    // 未执行分支和同一真实类型的别名仍贡献闭合根。最终计划无需保存根声明清单，
    // 直接以节点唯一性、冻结 key 和缺席路由验证编译收集的结果。
    let graph = &crate::graph::plan::load().graph;
    assert_eq!(graph.nodes.len(), 3);
    for service_type in [
        ServiceType::create::<Repository<User>>(),
        ServiceType::create::<Repository<Order>>(),
    ] {
        let keyed = ServiceIdentifier::new(Some(ServiceKey::Named("root".into())), service_type);
        assert!(graph.routes.contains_key(&keyed));
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.identifier.service_type == service_type)
                .count(),
            1
        );
        assert!(
            !graph
                .routes
                .contains_key(&ServiceIdentifier::from(service_type))
        );
    }
    for service_type in [
        ServiceType::create::<dyn Port>(),
        ServiceType::create::<Missing>(),
    ] {
        assert!(
            !graph
                .routes
                .contains_key(&ServiceIdentifier::from(service_type))
        );
    }
    let factory =
        &graph.routes[&ServiceIdentifier::from(ServiceType::create::<FactoryOnly<User>>())];
    assert!(matches!(
        graph.nodes[factory.provider].constructor,
        crate::graph::Constructor::Factory(_)
    ));
}
