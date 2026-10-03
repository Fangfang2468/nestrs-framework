//! 经实际协调器与 worker 验证混合槽位交付，以及弱请求能力绑定的真实 owner。

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use ahash::AHashMap;

use crate::{
    ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, DependencyLease, ErasedService, FactoryFuture,
        FactoryInputs, InputSlot, LazyInjection, LazyInputPlan, ServiceProjector,
        adapter::FactoryInvoker, project_bound, project_required,
    },
    graph::{
        AbsentInput, CompiledDependency, CompiledNode, Constructor, DependencyInput, NodePolicy,
        ValidatedGraph,
    },
    runtime::Runtime,
    service::{Injectable, ServiceIdentifier, ServiceSource, ServiceType},
};

fn identifier<T: Injectable + ?Sized>() -> ServiceIdentifier {
    ServiceIdentifier::new(None, ServiceType::create::<T>())
}

fn node<T: Injectable>(lifetime: ServiceLifetime, constructor: Constructor) -> CompiledNode {
    CompiledNode {
        identifier: identifier::<T>(),
        common: NodePolicy {
            lifetime,
            lazy: None,
            source: ServiceSource::new(file!(), line!(), 1),
            cleanup: None,
        },
        dependencies: Vec::new(),
        constructor,
        requires_scope: lifetime == ServiceLifetime::Scoped,
    }
}

fn lazy<T: Injectable + ?Sized>(
    consumer: &CompiledNode,
    slot: usize,
    target: usize,
    project: ServiceProjector,
) -> CompiledDependency {
    let slot = InputSlot::new(slot);
    CompiledDependency {
        slot,
        requested: identifier::<T>(),
        optional: false,
        label: None,
        input: DependencyInput::Lazy {
            plan: Arc::new(LazyInputPlan {
                provider: target,
                consumer: consumer.identifier.clone(),
                source: consumer.common.source,
                label: None,
                input: slot,
                project,
            }),
        },
    }
}

fn graph(nodes: Vec<CompiledNode>) -> Arc<ValidatedGraph> {
    // 本文件的 fixture 显式把依赖放在消费者前面；这里只形成去重反向索引，不编译图。
    let mut dependents = vec![Vec::new(); nodes.len()];
    for (consumer, node) in nodes.iter().enumerate() {
        for dependency in &node.dependencies {
            if let Some(target) = dependency.input.target() {
                assert!(target < consumer);
                if !dependents[target].contains(&consumer) {
                    dependents[target].push(consumer);
                }
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

fn service<T: Injectable>(lease: &DependencyLease) -> &T {
    // SAFETY: pointer 检查准确 T；返回借用绑定仍持有真实实例的 lease。
    unsafe { lease.pointer::<T>().unwrap().as_ref() }
}

#[tokio::test]
async fn worker_delivers_mixed_slots_for_classes_and_both_factory_kinds() {
    static BUILDS: AtomicUsize = AtomicUsize::new(0);
    struct Target(usize);
    struct Missing;
    trait MissingPort: Send + Sync {}
    trait Port: Send + Sync {
        fn serial(&self) -> usize;
    }
    impl Port for Target {
        fn serial(&self) -> usize {
            self.0
        }
    }
    struct Mixed {
        immediate: u32,
        first: LazyInjection<dyn Port>,
        second: LazyInjection<Target>,
    }
    fn class(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
        let immediate = *inputs.take::<u32>(InputSlot::new(0))?;
        let first = inputs.take_lazy(InputSlot::new(1))?;
        assert!(
            inputs
                .take_optional::<Missing>(InputSlot::new(2))?
                .is_none()
        );
        let second = inputs.take_lazy(InputSlot::new(3))?;
        assert!(
            inputs
                .take_optional_lazy::<dyn MissingPort>(InputSlot::new(4))?
                .is_none()
        );
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(Mixed {
            immediate,
            first,
            second,
        }))
    }
    fn factory(mut inputs: FactoryInputs<'_>) -> Result<ErasedService, ConstructionError> {
        let immediate = *inputs.take::<u32>(InputSlot::new(0))?;
        let first = inputs.take_lazy(InputSlot::new(1))?;
        assert!(
            inputs
                .take_optional::<Missing>(InputSlot::new(2))?
                .is_none()
        );
        let second = inputs.take_lazy(InputSlot::new(3))?;
        assert!(
            inputs
                .take_optional_lazy::<dyn MissingPort>(InputSlot::new(4))?
                .is_none()
        );
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(Mixed {
            immediate,
            first,
            second,
        }))
    }
    fn asynchronous(inputs: FactoryInputs<'_>) -> FactoryFuture<'_> {
        Box::pin(async move {
            tokio::task::yield_now().await;
            factory(inputs)
        })
    }
    for constructor in [
        Constructor::Class(class),
        Constructor::Factory(FactoryInvoker::Sync(factory)),
        Constructor::Factory(FactoryInvoker::Async(asynchronous)),
    ] {
        let before = BUILDS.load(Ordering::SeqCst);
        let mut consumer = node::<Mixed>(ServiceLifetime::Transient, constructor);
        consumer.dependencies = vec![
            CompiledDependency {
                slot: InputSlot::new(0),
                requested: identifier::<u32>(),
                optional: false,
                label: None,
                input: DependencyInput::Immediate {
                    target: 0,
                    project: project_required::<u32>,
                },
            },
            lazy::<dyn Port>(&consumer, 1, 1, |slot, value, output| {
                project_bound::<Target, dyn Port>(slot, value, output, |target| target)
            }),
            CompiledDependency {
                slot: InputSlot::new(2),
                requested: identifier::<Missing>(),
                optional: true,
                label: None,
                input: DependencyInput::Absent(AbsentInput::Immediate),
            },
            lazy::<Target>(&consumer, 3, 1, project_required::<Target>),
            CompiledDependency {
                slot: InputSlot::new(4),
                requested: identifier::<dyn MissingPort>(),
                optional: true,
                label: None,
                input: DependencyInput::Absent(AbsentInput::Lazy),
            },
        ];
        let plan = graph(vec![
            node::<u32>(
                ServiceLifetime::Singleton,
                Constructor::Class(|inputs| {
                    inputs.ensure_all_consumed()?;
                    Ok(ErasedService::new(73_u32))
                }),
            ),
            node::<Target>(
                ServiceLifetime::Transient,
                Constructor::Class(|inputs| {
                    inputs.ensure_all_consumed()?;
                    Ok(ErasedService::new(Target(
                        BUILDS.fetch_add(1, Ordering::SeqCst),
                    )))
                }),
            ),
            consumer,
        ]);
        let (runtime, root) = Runtime::start(plan, 1);
        let lease = runtime.resolve(&root, 2).await.unwrap();
        let mixed = service::<Mixed>(&lease);
        assert_eq!(mixed.immediate, 73);
        assert_eq!(
            BUILDS.load(Ordering::SeqCst),
            before,
            "包装句柄不能构造目标"
        );
        let (first, same_field) = tokio::join!(mixed.first.get(), mixed.first.get());
        assert_eq!(first.unwrap().serial(), same_field.unwrap().serial());
        let second = mixed.second.get().await.unwrap();
        assert_ne!(mixed.first.get().await.unwrap().serial(), second.0);
        assert_eq!(
            BUILDS.load(Ordering::SeqCst),
            before + 2,
            "两个字段各有一次 Transient occurrence"
        );
        runtime.close(&root).await.unwrap();
    }
}

#[tokio::test]
async fn worker_lazy_capability_follows_actual_owner_without_keeping_scopes_alive() {
    struct Target;
    struct Consumer {
        target: LazyInjection<Target>,
    }
    for lifetime in [ServiceLifetime::Singleton, ServiceLifetime::Scoped] {
        let mut consumer = node::<Consumer>(
            lifetime,
            Constructor::Class(|mut inputs| {
                let target = inputs.take_lazy(InputSlot::new(0))?;
                inputs.ensure_all_consumed()?;
                Ok(ErasedService::new(Consumer { target }))
            }),
        );
        consumer.dependencies = vec![lazy::<Target>(&consumer, 0, 0, project_required::<Target>)];
        let (runtime, root) = Runtime::start(
            graph(vec![
                node::<Target>(
                    ServiceLifetime::Transient,
                    Constructor::Class(|inputs| {
                        inputs.ensure_all_consumed()?;
                        Ok(ErasedService::new(Target))
                    }),
                ),
                consumer,
            ]),
            1,
        );
        let first_scope = runtime.create_scope();
        let second_scope = runtime.create_scope();
        let first = runtime.resolve(&first_scope, 1).await.unwrap();
        let second = runtime.resolve(&second_scope, 1).await.unwrap();
        assert_eq!(
            first.ptr_eq(&second),
            lifetime == ServiceLifetime::Singleton
        );
        let weak_scope = Arc::downgrade(&first_scope.data);
        runtime.close(&first_scope).await.unwrap();
        drop(first_scope);
        assert!(
            weak_scope.upgrade().is_none(),
            "逃逸的消费者不能通过 lazy 能力保活 scope"
        );
        let first_service = service::<Consumer>(&first);
        let second_service = service::<Consumer>(&second);
        if lifetime == ServiceLifetime::Singleton {
            // 请求由已销毁的 scope 发起，但真实 owner 是 root，首次 Lazy 获取仍应成功。
            first_service.target.get().await.unwrap();
            assert_eq!(root.data.journal.lock().unwrap().len(), 2);
            assert!(second_scope.data.journal.lock().unwrap().is_empty());
        } else {
            assert!(first_service.target.get().await.is_err());
            second_service.target.get().await.unwrap();
            assert_eq!(second_scope.data.journal.lock().unwrap().len(), 2);
            assert!(root.data.journal.lock().unwrap().is_empty());
        }
        runtime.close(&second_scope).await.unwrap();
        drop(second_scope);
        // 已取得的 token 仍由逃逸消费者保活，逻辑关闭不撤销已经交付的结果。
        second_service.target.get().await.unwrap();
        runtime.close(&root).await.unwrap();
    }
}
