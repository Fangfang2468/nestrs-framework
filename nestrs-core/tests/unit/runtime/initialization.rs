//! 服务声明级初始化策略只影响预热入口，不改变依赖构造、缓存与 owner 边界。

use ahash::AHashMap;
use std::sync::Arc;

use super::{Owner, Runtime};
use crate::{
    InitializationMode, ServiceLifetime,
    activation::{ConstructionInputs, ErasedService, InputSlot, prepare_required},
    graph::NodePolicy,
    graph::{CompiledDependency, CompiledNode, Constructor, DependencyInput, ValidatedGraph},
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};

fn identifier(index: usize) -> ServiceIdentifier {
    ServiceIdentifier::new(
        Some(ServiceKey::Indexed(index)),
        ServiceType::create::<u32>(),
    )
}

fn node(index: usize, lifetime: ServiceLifetime, lazy: Option<bool>) -> CompiledNode {
    CompiledNode {
        identifier: identifier(index),
        common: NodePolicy {
            lifetime,

            lazy,
            source: ServiceSource::new(file!(), line!(), column!()),
            cleanup: None,
        },
        dependencies: vec![],
        constructor: Constructor::Class(|inputs| {
            inputs.ensure_all_consumed()?;
            Ok(ErasedService::new(42_u32))
        }),
        requires_scope: lifetime == ServiceLifetime::Scoped,
    }
}

fn graph(nodes: Vec<CompiledNode>) -> Arc<ValidatedGraph> {
    let mut dependents = vec![vec![]; nodes.len()];
    for (consumer, node) in nodes.iter().enumerate() {
        for dependency in &node.dependencies {
            if let Some(target) = dependency.input.target() {
                dependents[target].push(consumer);
            }
        }
    }
    Arc::new(ValidatedGraph {
        topological_order: (0..nodes.len()).collect(),
        nodes,
        dependents,
        routes: AHashMap::new(),
    })
}

fn published(owner: &Owner) -> Vec<usize> {
    let mut result: Vec<_> = owner
        .data
        .journal
        .lock()
        .unwrap()
        .iter()
        .map(|entry| entry.provider)
        .collect();
    result.sort_unstable();
    result
}

#[tokio::test]
async fn shared_plan_allows_each_root_default_without_merging_instances() {
    // 同一冻结计划保留三态策略，不允许提前按某个 root 的默认值删掉声明。
    let graph = graph(vec![
        node(0, ServiceLifetime::Singleton, None),
        node(1, ServiceLifetime::Singleton, Some(true)),
        node(2, ServiceLifetime::Singleton, Some(false)),
    ]);
    let (lazy_runtime, lazy_root) = Runtime::start(graph.clone(), 2);
    let (eager_runtime, eager_root) = Runtime::start(graph.clone(), 2);
    lazy_runtime
        .warm_up(
            &lazy_root,
            ServiceLifetime::Singleton,
            InitializationMode::Lazy,
        )
        .await
        .unwrap();
    eager_runtime
        .warm_up(
            &eager_root,
            ServiceLifetime::Singleton,
            InitializationMode::Eager,
        )
        .await
        .unwrap();
    assert_eq!(published(&lazy_root), [2]);
    assert_eq!(published(&eager_root), [0, 2]);
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.nodes[0].common.lazy, None);
    let first = lazy_runtime.resolve(&lazy_root, 2).await.unwrap();
    let second = eager_runtime.resolve(&eager_root, 2).await.unwrap();
    assert!(!first.ptr_eq(&second));
    // 跳过预热的声明仍有正常查询和 Singleton 缓存能力。
    let deferred = lazy_runtime.resolve(&lazy_root, 1).await.unwrap();
    assert!(deferred.ptr_eq(&lazy_runtime.resolve(&lazy_root, 1).await.unwrap()));
    assert_eq!(published(&lazy_root), [1, 2]);
    lazy_runtime.close(&lazy_root).await.unwrap();
    eager_runtime.close(&eager_root).await.unwrap();
}

#[tokio::test]
async fn scopes_honor_declaration_overrides_and_transients_are_never_preheat_roots() {
    let graph = graph(vec![
        node(0, ServiceLifetime::Scoped, None),
        node(1, ServiceLifetime::Scoped, Some(true)),
        node(2, ServiceLifetime::Scoped, Some(false)),
        node(3, ServiceLifetime::Transient, Some(false)),
    ]);
    let (runtime, root) = Runtime::start(graph, 3);
    let first = runtime.create_scope();
    let second = runtime.create_scope();
    assert!(published(&first).is_empty());
    assert!(published(&second).is_empty());
    runtime
        .warm_up(&first, ServiceLifetime::Scoped, InitializationMode::Eager)
        .await
        .unwrap();
    assert_eq!(published(&first), [0, 2]);
    assert!(published(&second).is_empty());
    assert!(published(&root).is_empty());
    // 重复预热必须复用该 scope 的缓存，而不产生额外实例。
    runtime
        .warm_up(&first, ServiceLifetime::Scoped, InitializationMode::Eager)
        .await
        .unwrap();
    assert_eq!(published(&first), [0, 2]);
    let deferred_first = runtime.resolve(&first, 1).await.unwrap();
    let deferred_second = runtime.resolve(&second, 1).await.unwrap();
    assert!(!deferred_first.ptr_eq(&deferred_second));
    // 即使内部误将 Transient 作为预热类型，也不能凭 lazy(false) 制造无消费者实例。
    runtime
        .warm_up(
            &first,
            ServiceLifetime::Transient,
            InitializationMode::Eager,
        )
        .await
        .unwrap();
    assert_eq!(published(&first), [0, 1, 2]);
    let transient_first = runtime.resolve(&first, 3).await.unwrap();
    let transient_second = runtime.resolve(&first, 3).await.unwrap();
    assert!(!transient_first.ptr_eq(&transient_second));
    assert_eq!(published(&first), [0, 1, 2, 3, 3]);
    runtime.close(&first).await.unwrap();
    runtime.close(&second).await.unwrap();
    runtime.close(&root).await.unwrap();
}

#[tokio::test]
async fn an_ordinary_dependency_constructs_a_lazy_provider_before_its_eager_consumer() {
    let dependency = node(0, ServiceLifetime::Singleton, Some(true));
    let mut consumer = node(1, ServiceLifetime::Singleton, Some(false));
    consumer.dependencies.push(CompiledDependency {
        slot: InputSlot::new(0),
        requested: identifier(0),
        optional: false,
        input: DependencyInput::Immediate {
            target: 0,
            prepare: prepare_required::<u32>,
        },
        label: Some("ordinary"),
    });
    consumer.constructor = Constructor::Class(|mut inputs: ConstructionInputs| {
        let dependency = inputs.take::<u32>(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(*dependency + 1))
    });
    let (runtime, root) = Runtime::start(graph(vec![dependency, consumer]), 1);
    runtime
        .warm_up(&root, ServiceLifetime::Singleton, InitializationMode::Lazy)
        .await
        .unwrap();
    assert_eq!(published(&root), [0, 1]);
    let consumer = runtime.resolve(&root, 1).await.unwrap();
    // SAFETY: lease 保活准确 u32 实例，pointer 检查与声明一致的真实类型。
    assert_eq!(unsafe { *consumer.pointer::<u32>().unwrap().as_ref() }, 43);
    runtime.close(&root).await.unwrap();
}
