//! 工具链生成最终执行节点与接口路由；投影复用同一实例，不增加生产节点。
use crate::activation::adapter::{Constructor, FactoryInvoker};
use crate::service::{ServiceIdentifier, ServiceKey, ServiceType};
use nestrs::{bind, factory, injectable};

trait Greeter: Send + Sync {
    fn greet(&self) -> &'static str;
}

struct GreeterService;

#[injectable]
struct Repository {
    #[value("registry")]
    label: String,
}

#[factory(key = "greeting")]
fn make_greeter() -> GreeterService {
    GreeterService
}

#[bind]
impl Greeter for GreeterService {
    fn greet(&self) -> &'static str {
        "hello"
    }
}

#[test]
fn reflect_contains_execution_nodes_and_shared_trait_routes() {
    let graph = &crate::graph::plan::load().graph;
    let repository = &graph.nodes
        [graph.routes[&ServiceIdentifier::from(ServiceType::create::<Repository>())].provider];
    assert!(repository.dependencies.is_empty());
    assert!(repository.common.cleanup.is_none());
    let Constructor::Class(constructor) = repository.constructor else {
        panic!("class adapter")
    };
    let erased = constructor(crate::activation::ConstructionInputs::empty()).unwrap();
    let repository = match erased.downcast::<Repository>() {
        Ok(value) => value,
        Err(_) => panic!("类型化构造入口必须保留准确 concrete 类型"),
    };
    assert_eq!(repository.label, "registry");

    let key = Some(ServiceKey::Named("greeting".into()));
    let concrete = &graph.routes
        [&ServiceIdentifier::new(key.clone(), ServiceType::create::<GreeterService>())];
    let interface =
        &graph.routes[&ServiceIdentifier::new(key, ServiceType::create::<dyn Greeter>())];
    assert_eq!(concrete.provider, interface.provider);
    assert!(interface.projection.is_some());
    assert!(matches!(
        graph.nodes[concrete.provider].constructor,
        Constructor::Factory(FactoryInvoker::Sync(_))
    ));
    assert_eq!(graph.nodes.len(), 2, "接口投影不额外生产实例节点");
    assert_eq!(Greeter::greet(&GreeterService), "hello");
}
