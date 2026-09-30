//! 深层延迟依赖的展开和逃逸 lease 释放回归。相同 Rust 类型的不同 provider 模拟长链，
//! 不依赖 Rust 类型嵌套，真正测量框架算法的栈使用。

use crate::{
    ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, InputSlot, LazyInjection,
        prepare_lazy_optional, prepare_optional,
    },
    graph::{CompiledDependency, CompiledNode, Constructor, ValidatedGraph},
    registration::provider::ProviderCommon,
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

static DROPS: AtomicUsize = AtomicUsize::new(0);
struct Chain {
    next: Option<LazyInjection<Chain>>,
}
impl Drop for Chain {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}
fn construct(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    let next = inputs.take_optional_lazy::<Chain>(InputSlot::new(0))?;
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(Chain { next }))
}

#[test]
fn ten_thousand_deferred_nodes_construct_close_and_release_on_small_stack() {
    const DEPTH: usize = 10_000;
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async {
                let source = ServiceSource::new(file!(), line!(), 1);
                let mut nodes = Vec::new();
                let mut dependents = vec![Vec::new(); DEPTH];
                for index in 0..DEPTH {
                    let target = (index + 1 < DEPTH).then_some(index + 1);
                    if let Some(target) = target {
                        dependents[target].push(index);
                    }
                    nodes.push(CompiledNode {
                        identifier: ServiceIdentifier::new(
                            Some(ServiceKey::Indexed(index)),
                            ServiceType::create::<Chain>(),
                        ),
                        common: ProviderCommon {
                            lifetime: ServiceLifetime::Transient,
                            primary: false,
                            source,
                            cleanup: None,
                        },
                        dependencies: vec![CompiledDependency {
                            slot: InputSlot::new(0),
                            requested: ServiceIdentifier::new(
                                Some(ServiceKey::Indexed(index + 1)),
                                ServiceType::create::<Chain>(),
                            ),
                            optional: true,
                            target,
                            prepare: prepare_optional::<Chain>,
                            lazy: Some(prepare_lazy_optional::<Chain>),
                            label: Some("next"),
                        }],
                        constructor: Constructor::Class(construct),
                        requires_scope: false,
                    });
                }
                let graph = Arc::new(ValidatedGraph {
                    nodes,
                    dependents,
                    topological_order: (0..DEPTH).rev().collect(),
                    routes: HashMap::new(),
                });
                let (container, owner) = super::super::Runtime::start(graph, 1);
                let lease = container.resolve(&owner, 0).await.unwrap();
                // SAFETY: 当前 lease 保活准确类型的稳定实例，之后每一步由延迟字段的强 lease 保活。
                let mut current = unsafe { lease.pointer::<Chain>().unwrap().as_ref() };
                let mut count = 1;
                while let Some(next) = &current.next {
                    current = next.get().await.unwrap();
                    count += 1;
                }
                assert_eq!(count, DEPTH);
                container.close(&owner).await.unwrap();
                assert_eq!(
                    DROPS.load(Ordering::SeqCst),
                    0,
                    "逃逸首节点应保活完整已初始化链"
                );
                drop(owner);
                drop(container);
                drop(lease);
                assert_eq!(
                    DROPS.load(Ordering::SeqCst),
                    DEPTH,
                    "最后一个 lease 应迭代释放完整链且恰好一次"
                );
            });
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn an_accepted_lazy_request_reports_runtime_exit_instead_of_waiting_forever() {
    use crate::{
        activation::{FactoryFuture, FactoryInputs, prepare_lazy_required, prepare_required},
        registration::provider::FactoryInvoker,
    };
    use std::time::Duration;

    static STARTED: AtomicUsize = AtomicUsize::new(0);
    struct PendingTarget;
    struct Consumer {
        target: LazyInjection<PendingTarget>,
    }
    fn consumer(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
        let target = inputs.take_lazy::<PendingTarget>(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(Consumer { target }))
    }
    fn pending(inputs: FactoryInputs<'_>) -> FactoryFuture<'_> {
        Box::pin(async move {
            inputs.ensure_all_consumed()?;
            STARTED.fetch_add(1, Ordering::SeqCst);
            std::future::pending().await
        })
    }

    let source = ServiceSource::new(file!(), line!(), 1);
    let common = || ProviderCommon {
        lifetime: ServiceLifetime::Transient,
        primary: false,
        source,
        cleanup: None,
    };
    let target = ServiceIdentifier::new(None, ServiceType::create::<PendingTarget>());
    let graph = Arc::new(ValidatedGraph {
        nodes: vec![
            CompiledNode {
                identifier: ServiceIdentifier::new(None, ServiceType::create::<Consumer>()),
                common: common(),
                dependencies: vec![CompiledDependency {
                    slot: InputSlot::new(0),
                    requested: target.clone(),
                    optional: false,
                    target: Some(1),
                    prepare: prepare_required::<PendingTarget>,
                    lazy: Some(prepare_lazy_required::<PendingTarget>),
                    label: Some("target"),
                }],
                constructor: Constructor::Class(consumer),
                requires_scope: false,
            },
            CompiledNode {
                identifier: target,
                common: common(),
                dependencies: vec![],
                constructor: Constructor::Factory(FactoryInvoker::Async(pending)),
                requires_scope: false,
            },
        ],
        dependents: vec![vec![], vec![0]],
        topological_order: vec![1, 0],
        routes: HashMap::new(),
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (container, owner, lease) = runtime.block_on(async {
        let (container, owner) = super::super::Runtime::start(graph, 1);
        let lease = container.resolve(&owner, 0).await.unwrap();
        // SAFETY: 外层 lease 保活准确的 Consumer；没有扩展借用，也没有移走服务值。
        let service = unsafe { lease.pointer::<Consumer>().unwrap().as_ref() };
        tokio::time::timeout(Duration::from_secs(2), async {
            tokio::select! {
                result = service.target.get() => panic!("目标工厂应保持 pending，实际：{}", result.is_ok()),
                () = async {
                    while STARTED.load(Ordering::SeqCst) == 0 {
                        tokio::task::yield_now().await;
                    }
                } => {}
            }
        }).await.expect("延迟目标应进入受跟踪的构造 worker");
        // 此时 get 的等待被取消；槽位仍保留同一个已接受请求的 watch 接收端。
        (container, owner, lease)
    });
    drop(runtime);

    let next_runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    next_runtime.block_on(async {
        // SAFETY: runtime 的消失不会释放仍由当前 lease 保活的已发布消费者。
        let service = unsafe { lease.pointer::<Consumer>().unwrap().as_ref() };
        let result = tokio::time::timeout(Duration::from_secs(2), service.target.get())
            .await
            .expect("原协调器已销毁全部 sender，新的等待必须立即得到关闭错误");
        assert!(result.err().unwrap().to_string().contains("协调器已停止"));
        assert_eq!(
            STARTED.load(Ordering::SeqCst),
            1,
            "重新等待不能提交另一份 Transient 请求"
        );
    });
    drop(owner);
    drop(container);
    drop(lease);
}
