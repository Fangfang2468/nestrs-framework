//! 延迟槽位的运行时等待协议，以及深层依赖展开和逃逸 lease 释放回归。
//! 长链用相同 Rust 类型的不同 provider 模拟，不依赖 Rust 类型嵌套，真正测量算法的栈使用。

use crate::{
    ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, InputSlot, LazyInjection,
        LazyInputPlan, prepare_lazy_optional, project_required,
    },
    graph::NodePolicy,
    graph::{
        AbsentInput, CompiledDependency, CompiledNode, Constructor, DependencyInput, ValidatedGraph,
    },
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};
use ahash::AHashMap;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
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

struct Reports(u32);
struct ReportsConsumer;

/// 经真实输入准备路径取得句柄；owner 自身实现请求协议，不另建测试 resolver。
fn reports_token(owner: &Arc<crate::runtime::owner::OwnerData>) -> LazyInjection<Reports> {
    reports_token_with_plan(owner, reports_plan())
}

fn reports_plan() -> Arc<LazyInputPlan> {
    Arc::new(LazyInputPlan {
        provider: 11,
        consumer: ServiceIdentifier::new(None, ServiceType::create::<ReportsConsumer>()),
        source: ServiceSource::new("business/reports.rs", 34, 5),
        label: Some("reports"),
        input: InputSlot::new(0),
        project: project_required::<Reports>,
    })
}

fn reports_token_with_plan(
    owner: &Arc<crate::runtime::owner::OwnerData>,
    plan: Arc<LazyInputPlan>,
) -> LazyInjection<Reports> {
    use crate::activation::{ActivationPreparation, LazyDependency, prepare_lazy_required};
    let resolver = Arc::downgrade(owner);
    let slot = InputSlot::new(0);
    let dependency = LazyDependency {
        resolver,
        check_wait_allowed: super::check_wait_allowed,
        plan,
    };
    let mut preparation = ActivationPreparation::new(1);
    preparation
        .prepare_lazy(slot, prepare_lazy_required::<Reports>, Some(dependency))
        .unwrap();
    let (mut inputs, leases) = preparation.finish_class().unwrap();
    assert!(leases.is_empty());
    let token = inputs.take_lazy::<Reports>(slot).unwrap();
    inputs.ensure_all_consumed().unwrap();
    token
}

#[tokio::test]
async fn lazy_edges_skip_activation_waits_but_order_cleanup_after_the_consumer() {
    use crate::activation::{adapter::CleanupFuture, prepare_lazy_required};
    use std::sync::Mutex;
    static TARGET_BUILDS: AtomicUsize = AtomicUsize::new(0);
    static CLEANUPS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

    struct Target;
    struct Consumer {
        target: LazyInjection<Target>,
    }
    fn target(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
        inputs.ensure_all_consumed()?;
        TARGET_BUILDS.fetch_add(1, Ordering::SeqCst);
        Ok(ErasedService::new(Target))
    }
    fn consumer(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
        let target = inputs.take_lazy(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(Consumer { target }))
    }
    fn cleanup_consumer() -> CleanupFuture {
        Box::pin(async {
            // 必须等整个异步 hook 完成后才开始依赖 hook，不能只检查提交顺序。
            tokio::task::yield_now().await;
            CLEANUPS.lock().unwrap().push("consumer");
        })
    }
    fn cleanup_target() -> CleanupFuture {
        Box::pin(async {
            assert_eq!(*CLEANUPS.lock().unwrap(), ["consumer"]);
            CLEANUPS.lock().unwrap().push("target");
        })
    }
    let source = ServiceSource::new(file!(), line!(), 1);
    let consumer_id = ServiceIdentifier::new(None, ServiceType::create::<Consumer>());
    let target_id = ServiceIdentifier::new(None, ServiceType::create::<Target>());
    let graph = Arc::new(ValidatedGraph {
        nodes: vec![
            CompiledNode {
                identifier: consumer_id.clone(),
                common: NodePolicy {
                    lifetime: ServiceLifetime::Singleton,
                    lazy: None,
                    source,
                    cleanup: Some(cleanup_consumer),
                },
                dependencies: vec![CompiledDependency {
                    slot: InputSlot::new(0),
                    requested: target_id.clone(),
                    optional: false,
                    input: DependencyInput::Lazy {
                        prepare: prepare_lazy_required::<Target>,
                        plan: Arc::new(LazyInputPlan {
                            provider: 1,
                            consumer: consumer_id,
                            source,
                            label: Some("target"),
                            input: InputSlot::new(0),
                            project: project_required::<Target>,
                        }),
                    },
                    label: Some("target"),
                }],
                constructor: Constructor::Class(consumer),
                requires_scope: false,
            },
            CompiledNode {
                identifier: target_id,
                common: NodePolicy {
                    lifetime: ServiceLifetime::Singleton,
                    lazy: None,
                    source,
                    cleanup: Some(cleanup_target),
                },
                dependencies: vec![],
                constructor: Constructor::Class(target),
                requires_scope: false,
            },
        ],
        dependents: vec![vec![], vec![0]],
        topological_order: vec![1, 0],
        routes: AHashMap::new(),
    });
    let (runtime, owner) = super::super::Runtime::start(graph, 1);
    let lease = runtime.resolve(&owner, 0).await.unwrap();
    assert_eq!(
        TARGET_BUILDS.load(Ordering::SeqCst),
        0,
        "延迟目标不阻塞消费者发布"
    );
    // SAFETY: lease 保活准确的 Consumer 实例，运行期发布前已经核对真实类型。
    let service = unsafe { lease.pointer::<Consumer>().unwrap().as_ref() };
    service.target.get().await.unwrap();
    assert_eq!(TARGET_BUILDS.load(Ordering::SeqCst), 1);
    assert_eq!(
        owner
            .data
            .journal
            .lock()
            .unwrap()
            .iter()
            .map(|entry| entry.provider)
            .collect::<Vec<_>>(),
        [0, 1],
        "目标确实晚于消费者发布，不能单靠发布时间逆序得到正确清理顺序"
    );
    runtime.close(&owner).await.unwrap();
    assert_eq!(*CLEANUPS.lock().unwrap(), ["consumer", "target"]);
}

#[tokio::test]
async fn worker_wait_rejection_does_not_poison_or_resubmit_a_lazy_occurrence() {
    use super::IN_ACTIVATION;
    use crate::{
        activation::{DependencyLease, ReleaseDomain},
        runtime::{handle::Command, owner::OwnerData},
    };
    use std::{future::poll_fn, task::Poll};
    use tokio::sync::mpsc;

    let (commands, mut requests) = mpsc::unbounded_channel();
    let owner = OwnerData::new(7, commands.downgrade());
    let token = reports_token(&owner);

    let error = IN_ACTIVATION.scope((), token.get()).await.err().unwrap();
    assert!(error.to_string().contains("服务构造期间不能首次获取"));
    assert!(error.to_string().contains("reports"));
    assert!(requests.try_recv().is_err(), "拒绝等待不能提前提交初始化");

    let mut first = Box::pin(token.get());
    poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let Command::ResolveLazy {
        owner: owner_id,
        provider,
        waiter,
    } = requests.try_recv().expect("普通调用必须提交延迟请求")
    else {
        panic!("延迟字段必须使用 ResolveLazy 协议");
    };
    assert_eq!(owner_id, owner.id);
    assert_eq!(provider, 11);

    // 此时第一个调用仍持有 OnceCell 的初始化权。若把保护仅搬进 resolve，
    // 后来的构造 worker 会先等待 OnceCell，永远没有机会检查许可。
    let mut blocked = Box::pin(IN_ACTIVATION.scope((), token.get()));
    let error = poll_fn(|cx| match blocked.as_mut().poll(cx) {
        Poll::Ready(result) => Poll::Ready(result.err().expect("构造 worker 必须拒绝等待")),
        Poll::Pending => panic!("构造 worker 的拒绝必须立即发生，不能等待其他调用完成"),
    })
    .await;
    assert!(error.to_string().contains("服务构造期间不能首次获取"));
    drop(blocked);

    // 取消首个调用只放弃它自己的等待；后来的正常调用仍连接同一 watch 槽位，
    // 不能再次提交 ResolveLazy。对 Transient 而言，这就是保持单次 occurrence 的边界。
    drop(first);
    let mut resumed = Box::pin(token.get());
    poll_fn(|cx| {
        assert!(resumed.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(requests.try_recv().is_err(), "接续等待不能重复提交初始化");

    let lease = DependencyLease::new(
        ErasedService::new(Reports(42)),
        vec![],
        ReleaseDomain::new(),
    );
    // 与协调器相同：先建立 owner 持有关系，再发布 Ready。这里直接驱动命令协议，
    // 无需计时 sleep、全局计数器或后台任务调度先后假设。
    owner.publish(provider, lease.clone());
    assert!(waiter.send(Some(Ok(lease))).is_ok());
    let value = resumed.await.expect("此前的等待拒绝不得污染最终结果");
    assert_eq!(value.0, 42);
    let cached = IN_ACTIVATION.scope((), token.get()).await.unwrap();
    assert!(
        std::ptr::eq(value, cached),
        "Ready 快路在构造上下文中也可使用"
    );
    assert!(requests.try_recv().is_err(), "读取 Ready 不再提交请求");
}

#[tokio::test]
async fn accepted_result_can_be_resumed_after_owner_close_and_drop() {
    use super::IN_ACTIVATION;
    use crate::{
        activation::{DependencyLease, ReleaseDomain},
        runtime::{handle::Command, owner::OwnerData},
    };
    use std::{future::poll_fn, task::Poll};
    use tokio::sync::mpsc;

    let (commands, mut requests) = mpsc::unbounded_channel();
    let owner = OwnerData::new(7, commands.downgrade());
    let weak_owner = Arc::downgrade(&owner);
    let token = reports_token(&owner);
    assert_eq!(Arc::strong_count(&owner), 1, "延迟字段只能弱持有 owner");

    let mut first = Box::pin(token.get());
    poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let Command::ResolveLazy {
        provider, waiter, ..
    } = requests.try_recv().unwrap()
    else {
        panic!("必须提交延迟请求");
    };
    drop(first);

    // owner 进入关闭后继续排空已接受任务。字段只需连接已保存的 receiver，不能再
    // 检查 owner 的 OPEN 状态，也不能为同一个字段提交第二份 Transient occurrence。
    owner.mark_closing();
    let mut resumed = Box::pin(token.get());
    poll_fn(|cx| {
        assert!(resumed.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert!(requests.try_recv().is_err());
    drop(resumed);

    let lease = DependencyLease::new(
        ErasedService::new(Reports(42)),
        vec![],
        ReleaseDomain::new(),
    );
    owner.publish(provider, lease.clone());
    assert!(waiter.send(Some(Ok(lease))).is_ok());
    // 模拟已排空 owner 的安全释放：先清空 journal，再发布关闭结果。此测试目标没有
    // cleanup hook，watch 中保留的真实 lease 仍须支撑取消后的首次类型化交付。
    drop(owner.next_cleanup());
    owner.complete_close(Ok(()));
    drop(owner);
    assert!(weak_owner.upgrade().is_none());
    drop(waiter);
    drop(commands);

    // 等待许可独立于 owner 生命周期。即便结果已经进入 watch，字段尚未完成类型化
    // 交付时也保持原先的 worker 拒绝语义，不因弱 owner 消失而误报另一种错误。
    let error = IN_ACTIVATION.scope((), token.get()).await.err().unwrap();
    assert!(error.to_string().contains("服务构造期间不能首次获取"));
    let value = token
        .get()
        .await
        .expect("已接受的结果不依赖 owner 继续存活");
    assert_eq!(value.0, 42);
    assert!(std::ptr::eq(value, token.get().await.unwrap()));
    assert!(requests.try_recv().is_err());
}

#[tokio::test]
async fn unrequested_fields_reject_a_closing_or_dropped_owner() {
    use crate::runtime::owner::OwnerData;
    use tokio::sync::mpsc;

    let (commands, mut requests) = mpsc::unbounded_channel();
    let owner = OwnerData::new(7, commands.downgrade());
    let plan = reports_plan();
    let closing_token = reports_token_with_plan(&owner, plan.clone());
    let dropped_token = reports_token_with_plan(&owner, plan);
    assert_eq!(Arc::strong_count(&owner), 1);
    owner.mark_closing();
    let error = closing_token.get().await.err().unwrap();
    assert!(error.to_string().contains("owner 已关闭或正在关闭"));
    assert!(error.to_string().contains("reports"));

    let weak_owner = Arc::downgrade(&owner);
    drop(owner);
    assert!(weak_owner.upgrade().is_none());
    let error = dropped_token.get().await.err().unwrap();
    assert!(error.to_string().contains("owner"));
    assert!(error.to_string().contains("reports"));
    assert!(error.to_string().contains("business/reports.rs:34:5"));
    assert!(error.to_string().contains("ReportsConsumer"));
    assert!(requests.try_recv().is_err(), "关闭后不能发起新的初始化");
}

#[tokio::test]
async fn shared_descriptors_submit_independent_owner_requests_for_each_field() {
    use crate::{
        ResolveError,
        activation::{DependencyLease, ReleaseDomain},
        runtime::{handle::Command, owner::OwnerData},
    };
    use std::{future::poll_fn, task::Poll};
    use tokio::sync::mpsc;

    let (commands, mut requests) = mpsc::unbounded_channel();
    let owner = OwnerData::new(7, commands.downgrade());
    let plan = reports_plan();
    let failed = reports_token_with_plan(&owner, plan.clone());
    let successful = reports_token_with_plan(&owner, plan.clone());
    assert_eq!(Arc::strong_count(&plan), 3);
    let mut failed_wait = Box::pin(failed.get());
    let mut successful_wait = Box::pin(successful.get());
    poll_fn(|cx| {
        assert!(failed_wait.as_mut().poll(cx).is_pending());
        assert!(successful_wait.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;

    // 相同描述与 owner 不能让两个字段共用 watch：Transient 每个消费字段都有独立
    // occurrence。Singleton/Scoped 是否共享服务仍由协调器生命周期缓存决定。
    for result in [
        Err(ResolveError::new("first field failed".to_owned())),
        Ok(DependencyLease::new(
            ErasedService::new(Reports(42)),
            vec![],
            ReleaseDomain::new(),
        )),
    ] {
        let Command::ResolveLazy {
            owner: id,
            provider,
            waiter,
        } = requests.try_recv().unwrap()
        else {
            panic!("每个首次访问字段必须独立提交 ResolveLazy")
        };
        assert_eq!((id, provider), (7, 11));
        waiter.send(Some(result)).unwrap();
    }
    assert!(
        failed_wait
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("first field failed")
    );
    assert_eq!(successful_wait.await.unwrap().0, 42);
    assert!(failed.get().await.is_err());
    assert_eq!(successful.get().await.unwrap().0, 42);
    assert!(requests.try_recv().is_err(), "缓存命中不能重新发送请求");
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
                        common: NodePolicy {
                            lifetime: ServiceLifetime::Transient,

                            lazy: None,
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
                            input: match target {
                                Some(provider) => DependencyInput::Lazy {
                                    prepare: prepare_lazy_optional::<Chain>,
                                    plan: Arc::new(LazyInputPlan {
                                        provider,
                                        consumer: ServiceIdentifier::new(
                                            Some(ServiceKey::Indexed(index)),
                                            ServiceType::create::<Chain>(),
                                        ),
                                        source,
                                        label: Some("next"),
                                        input: InputSlot::new(0),
                                        project: project_required::<Chain>,
                                    }),
                                },
                                None => DependencyInput::Absent(AbsentInput::Lazy(
                                    prepare_lazy_optional::<Chain>,
                                )),
                            },
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
                    routes: AHashMap::new(),
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
        activation::{FactoryFuture, FactoryInputs, prepare_lazy_required},
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
    let common = || NodePolicy {
        lifetime: ServiceLifetime::Transient,

        lazy: None,
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
                    input: DependencyInput::Lazy {
                        prepare: prepare_lazy_required::<PendingTarget>,
                        plan: Arc::new(LazyInputPlan {
                            provider: 1,
                            consumer: ServiceIdentifier::new(
                                None,
                                ServiceType::create::<Consumer>(),
                            ),
                            source,
                            label: Some("target"),
                            input: InputSlot::new(0),
                            project: project_required::<PendingTarget>,
                        }),
                    },
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
        routes: AHashMap::new(),
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
