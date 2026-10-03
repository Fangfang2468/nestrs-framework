use crate::activation::ConstructionInputs;
use crate::graph::Constructor;
use crate::lifetime::ServiceLifetime;
use crate::service::ServiceType;
use nestrs::injectable;
use std::marker::PhantomData;

struct User;
struct Entity;

// 这三个类型对应嵌套闭合泛型链：C -> B<u32> -> A<u32>。
// 它们刻意都使用本 crate 的类型，以便 #[injectable] 可以为开放泛型生成
// 类型化执行能力；本文件隔离验证最终闭合计划，实际运行由
// runtime_lifecycle 等端到端测试覆盖。
#[injectable]
struct A<T> {
    marker: PhantomData<T>,
}

#[injectable]
struct B<T> {
    #[inject]
    a: A<T>,
}

#[injectable]
struct C {
    #[inject]
    b: B<u32>,
}

async fn cleanup_repository() {}

/// 泛型 injectable 本身不应向编译器清单写入一个开放类型的 provider；具体类型的
/// provider 由工具链沿查询和依赖闭包确定。
#[injectable(lifetime = Transient, cleanup = "cleanup_repository")]
struct Repository<T> {
    #[value("generic-repository")]
    label: String,
    marker: PhantomData<T>,
}

#[injectable]
struct UserService {
    #[inject]
    repository: Repository<User>,
}

// 声明一个真实查询，使该闭合实例进入最终计划；不再调用运行期蓝图 API。
#[allow(dead_code)]
async fn query_entity(provider: &crate::ServiceProvider) {
    let _ = provider.get_service::<Repository<Entity>>().await;
}

fn node<T: Send + Sync + 'static>() -> &'static crate::graph::CompiledNode {
    crate::graph::plan::CompiledApplication::load()
        .graph
        .nodes
        .iter()
        .find(|node| node.identifier.service_type == ServiceType::create::<T>())
        .unwrap()
}

#[test]
fn generic_query_freezes_exact_execution_type_lifetime_and_cleanup() {
    let direct = node::<Repository<Entity>>();
    assert_eq!(direct.common.lifetime, ServiceLifetime::Transient);
    assert!(direct.dependencies.is_empty());
    drop(direct.common.cleanup.expect("generic cleanup")());
    let Constructor::Class(constructor) = direct.constructor else {
        panic!("generic class")
    };
    let erased = constructor(ConstructionInputs::empty()).unwrap();
    let repository = match erased.downcast::<Repository<Entity>>() {
        Ok(value) => value,
        Err(_) => panic!("closed Entity instance"),
    };
    assert_eq!(repository.label, "generic-repository");
}

#[test]
fn injected_generic_repository_uses_a_closed_target_without_runtime_materialization() {
    let graph = &crate::graph::plan::CompiledApplication::load().graph;
    let service = node::<UserService>();
    assert_eq!(service.dependencies.len(), 1);
    let input = &service.dependencies[0];
    assert_eq!(input.slot.index(), 0);
    assert_eq!(
        input.requested.service_type,
        ServiceType::create::<Repository<User>>()
    );
    let repository = &graph.nodes[input.input.target().unwrap()];
    assert_eq!(
        repository.identifier.service_type,
        ServiceType::create::<Repository<User>>()
    );
    let Constructor::Class(constructor) = repository.constructor else {
        panic!("generic class")
    };
    let erased = constructor(ConstructionInputs::empty()).unwrap();
    let repository = match erased.downcast::<Repository<User>>() {
        Ok(value) => value,
        Err(_) => panic!("closed User instance"),
    };
    assert_eq!(repository.label, "generic-repository");
    assert_eq!(graph.nodes.iter().filter(|node| node.identifier.service_type == ServiceType::create::<Repository<User>>()).count(), 1);
}

#[test]
fn nested_closed_generics_freeze_a_complete_execution_chain() {
    let graph = &crate::graph::plan::CompiledApplication::load().graph;
    let c = node::<C>();
    assert_eq!(c.dependencies.len(), 1);
    let b = &graph.nodes[c.dependencies[0].input.target().unwrap()];
    assert_eq!(b.identifier.service_type, ServiceType::create::<B<u32>>());
    assert_eq!(b.dependencies.len(), 1);
    let a = &graph.nodes[b.dependencies[0].input.target().unwrap()];
    assert_eq!(a.identifier.service_type, ServiceType::create::<A<u32>>());
    assert!(a.dependencies.is_empty());
    let Constructor::Class(constructor) = a.constructor else {
        panic!("leaf class")
    };
    assert!(
        constructor(ConstructionInputs::empty())
            .unwrap()
            .downcast::<A<u32>>()
            .is_ok()
    );
}
