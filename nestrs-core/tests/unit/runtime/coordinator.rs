//! 运行时状态与行为回归。直接检查内部任务表的用例用于证明任务会退役，
//! 其余测试通过真实 Tokio worker 验证并发、失败排空、深图与关闭契约。

#[path = "subscriptions.rs"]
mod subscriptions;

#[path = "panic_payloads.rs"]
mod panic_payloads;

#[path = "capacity.rs"]
mod capacity;

use ahash::AHashMap;
use std::{
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::Semaphore;

use super::super::{Runtime, TaskId, owner::CLOSED, task::TaskRequest};
use crate::{
    activation::adapter::FactoryInvoker,
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, FactoryFuture, FactoryInputs,
        InputSlot, project_required,
    },
    graph::NodePolicy,
    graph::{CompiledDependency, CompiledNode, Constructor, DependencyInput, ValidatedGraph},
    lifetime::ServiceLifetime,
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};

fn pending(request: TaskRequest) -> TaskId {
    match request {
        TaskRequest::Pending(task) => task,
        TaskRequest::Cached(_) => panic!("此请求应创建活跃任务"),
    }
}

fn node<T: Send + Sync + 'static>(
    index: usize,
    lifetime: ServiceLifetime,
    constructor: Constructor,
    dependencies: Vec<CompiledDependency>,
) -> CompiledNode {
    CompiledNode {
        identifier: ServiceIdentifier::new(
            Some(ServiceKey::Indexed(index)),
            ServiceType::create::<T>(),
        ),
        common: NodePolicy {
            lifetime,

            lazy: None,
            source: ServiceSource::new(file!(), line!(), 0),
            cleanup: None,
        },
        dependencies,
        constructor,
        requires_scope: lifetime == ServiceLifetime::Scoped,
    }
}

fn graph(nodes: Vec<CompiledNode>) -> Arc<ValidatedGraph> {
    let mut dependents = vec![Vec::new(); nodes.len()];
    for (consumer, node) in nodes.iter().enumerate() {
        for dependency in &node.dependencies {
            if let Some(provider) = dependency.input.target() {
                dependents[provider].push(consumer);
            }
        }
    }
    for consumers in &mut dependents {
        consumers.sort_unstable();
        consumers.dedup();
    }
    Arc::new(ValidatedGraph {
        topological_order: (0..nodes.len()).collect(),
        dependents,
        nodes,
        routes: AHashMap::new(),
    })
}

static SHARED_SUCCESS_CALLS: AtomicUsize = AtomicUsize::new(0);

#[tokio::test]
async fn closing_root_rejects_registration_and_completes_the_empty_scope() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let root = super::OwnerData::new(super::ROOT, sender.downgrade());
    let scope = super::OwnerData::new(1, sender.downgrade());
    let mut coordinator = super::Coordinator::new(graph(vec![]), root.clone(), receiver, 1);
    coordinator.begin_close(root, None);
    let (ready, registered) = tokio::sync::oneshot::channel();
    coordinator.handle_command(super::Command::Register {
        data: scope.clone(),
        ready,
    });
    assert!(registered.await.unwrap().is_err());
    assert!(!coordinator.owners.contains_key(&scope.id));
    assert_eq!(scope.status.load(Ordering::Acquire), CLOSED);
    assert!(scope.completed_close().unwrap().is_ok());
    coordinator.run().await;
}

fn counted_success(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    let value = SHARED_SUCCESS_CALLS.fetch_add(1, Ordering::SeqCst) as u32;
    Ok(ErasedService::new(value))
}

#[tokio::test]
async fn completed_shared_success_retires_tasks_and_preserves_owner_cache() {
    // 同一 scope 的并发请求必须合并；Singleton 还须跨 scope 合并，Scoped 则互相隔离。
    // 构造完成后任务表为空，但后续查询仍取得同一实例，证明缓存不再借用“完成任务”。
    for lifetime in [ServiceLifetime::Singleton, ServiceLifetime::Scoped] {
        SHARED_SUCCESS_CALLS.store(0, Ordering::SeqCst);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let root = super::OwnerData::new(super::ROOT, sender.downgrade());
        let first_scope = super::OwnerData::new(1, sender.downgrade());
        let second_scope = super::OwnerData::new(2, sender.downgrade());
        let mut coordinator = super::Coordinator::new(
            graph(vec![node::<u32>(
                0,
                lifetime,
                Constructor::Class(counted_success),
                vec![],
            )]),
            root.clone(),
            receiver,
            4,
        );
        coordinator.handle_command(super::Command::Register {
            data: first_scope.clone(),
            ready: tokio::sync::oneshot::channel().0,
        });
        coordinator.handle_command(super::Command::Register {
            data: second_scope.clone(),
            ready: tokio::sync::oneshot::channel().0,
        });
        let mut queries = Vec::new();
        for (query, owner) in [1, 1, 2].into_iter().enumerate() {
            let (waiter, result) = tokio::sync::oneshot::channel();
            coordinator.accept_resolution(owner, 0, (query as u64, waiter));
            queries.push(result);
        }
        let expected = if lifetime == ServiceLifetime::Singleton {
            1
        } else {
            2
        };
        assert_eq!(coordinator.tasks.len(), expected);
        assert_eq!(SHARED_SUCCESS_CALLS.load(Ordering::SeqCst), 0);
        coordinator.launch_ready();
        while let Some(completion) = coordinator.jobs.join_next_with_id().await {
            coordinator.handle_completion(completion);
        }
        assert_eq!(SHARED_SUCCESS_CALLS.load(Ordering::SeqCst), expected);
        assert!(coordinator.tasks.is_empty());
        assert!(
            coordinator
                .owners
                .values()
                .all(|owner| owner.active_tasks.is_empty())
        );

        let mut results = Vec::new();
        for query in queries {
            results.push(query.await.unwrap().unwrap());
        }
        assert!(results[0].ptr_eq(&results[1]));
        assert_eq!(
            results[0].ptr_eq(&results[2]),
            lifetime == ServiceLifetime::Singleton
        );
        let owner = if lifetime == ServiceLifetime::Singleton {
            super::ROOT
        } else {
            1
        };
        assert!(matches!(
            coordinator.owners[&owner].cache.get(&0),
            Some(super::CacheEntry::Ready(_))
        ));
        // 查询等待者收到结果时，真实 owner 的 journal 必须已经保活了该实例。
        assert_eq!(
            coordinator.owners[&owner]
                .data
                .journal
                .lock()
                .unwrap()
                .len(),
            1
        );

        let (waiter, cached) = tokio::sync::oneshot::channel();
        coordinator.accept_resolution(1, 0, (3, waiter));
        assert!(cached.await.unwrap().unwrap().ptr_eq(&results[0]));
        assert!(coordinator.tasks.is_empty());
        assert!(coordinator.jobs.is_empty());
        assert_eq!(SHARED_SUCCESS_CALLS.load(Ordering::SeqCst), expected);
        drop(results);
        coordinator.begin_close(root.clone(), None);
        coordinator.run().await;
        for owner in [root, first_scope, second_scope] {
            assert_eq!(owner.status.load(Ordering::Acquire), CLOSED);
            assert!(owner.journal.lock().unwrap().is_empty());
        }
    }
}

static SHARED_FAILURE_CALLS: AtomicUsize = AtomicUsize::new(0);

fn counted_failure(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    SHARED_FAILURE_CALLS.fetch_add(1, Ordering::SeqCst);
    Err(ConstructionError::FactoryFailed {
        provider: "counted_failure",
        provider_source: ServiceSource::new(file!(), line!(), 0),
        detail: "shared failure".to_owned(),
    })
}

#[tokio::test]
async fn completed_shared_failure_retires_tasks_without_retrying_or_publishing() {
    for lifetime in [ServiceLifetime::Singleton, ServiceLifetime::Scoped] {
        SHARED_FAILURE_CALLS.store(0, Ordering::SeqCst);
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let root = super::OwnerData::new(super::ROOT, sender.downgrade());
        let scope = super::OwnerData::new(1, sender.downgrade());
        let mut coordinator = super::Coordinator::new(
            graph(vec![node::<u32>(
                0,
                lifetime,
                Constructor::Class(counted_failure),
                vec![],
            )]),
            root.clone(),
            receiver,
            4,
        );
        coordinator.handle_command(super::Command::Register {
            data: scope.clone(),
            ready: tokio::sync::oneshot::channel().0,
        });
        let mut queries = Vec::new();
        for query in 0..3 {
            let (waiter, result) = tokio::sync::oneshot::channel();
            coordinator.accept_resolution(1, 0, (query, waiter));
            queries.push(result);
        }
        assert_eq!(coordinator.tasks.len(), 1);
        coordinator.launch_ready();
        let completion = coordinator.jobs.join_next_with_id().await.unwrap();
        coordinator.handle_completion(completion);
        let mut diagnostics = Vec::new();
        for query in queries {
            diagnostics.push(query.await.unwrap().err().unwrap().to_string());
        }
        assert!(diagnostics.iter().all(|detail| detail == &diagnostics[0]));
        assert!(diagnostics[0].contains("shared failure"));
        let owner = if lifetime == ServiceLifetime::Singleton {
            super::ROOT
        } else {
            1
        };
        assert!(matches!(
            coordinator.owners[&owner].cache.get(&0),
            Some(super::CacheEntry::Failed(_))
        ));
        assert!(coordinator.tasks.is_empty());
        assert!(
            coordinator
                .owners
                .values()
                .all(|owner| owner.active_tasks.is_empty())
        );
        assert!(root.journal.lock().unwrap().is_empty());
        assert!(scope.journal.lock().unwrap().is_empty());

        let (waiter, cached) = tokio::sync::oneshot::channel();
        coordinator.accept_resolution(1, 0, (3, waiter));
        assert_eq!(
            cached.await.unwrap().err().unwrap().to_string(),
            diagnostics[0]
        );
        assert_eq!(SHARED_FAILURE_CALLS.load(Ordering::SeqCst), 1);
        assert!(coordinator.tasks.is_empty());
        assert!(coordinator.jobs.is_empty());
        coordinator.begin_close(root.clone(), None);
        coordinator.run().await;
        assert_eq!(root.status.load(Ordering::Acquire), CLOSED);
        assert_eq!(scope.status.load(Ordering::Acquire), CLOSED);
    }
}

#[test]
fn failed_transient_tasks_are_retired_without_disposing_the_owner() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let graph = graph(vec![node::<Chain>(
        0,
        ServiceLifetime::Transient,
        Constructor::Class(chain_leaf),
        vec![],
    )]);
    let mut coordinator = super::Coordinator::new(
        graph,
        super::OwnerData::new(super::ROOT, sender.downgrade()),
        receiver,
        1,
    );
    for _ in 0..10_000 {
        let task = pending(coordinator.ensure_task(super::ROOT, 0, &mut vec![]));
        coordinator.settle(
            task,
            Err(crate::ResolveError::new("transient failure".into())),
        );
        assert!(coordinator.tasks.is_empty());
        assert!(coordinator.owners[&super::ROOT].active_tasks.is_empty());
    }
    let owner = &coordinator.owners[&super::ROOT];
    assert!(owner.active_tasks.is_empty());
    assert!(owner.cache.is_empty());
    assert!(owner.data.journal.lock().unwrap().is_empty());
}

#[tokio::test]
async fn retiring_a_failed_parent_still_drains_its_previously_accepted_children() {
    fn leaf(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(1_u32))
    }
    let dependencies = (0..2)
        .map(|provider| CompiledDependency {
            slot: InputSlot::new(provider),
            requested: ServiceIdentifier::new(
                Some(ServiceKey::Indexed(provider)),
                ServiceType::create::<u32>(),
            ),
            optional: false,
            input: DependencyInput::Immediate {
                target: provider,
                project: project_required::<u32>,
            },
            label: None,
        })
        .collect();
    let graph = graph(vec![
        node::<u32>(
            0,
            ServiceLifetime::Singleton,
            Constructor::Class(leaf),
            vec![],
        ),
        node::<u32>(
            1,
            ServiceLifetime::Transient,
            Constructor::Class(leaf),
            vec![],
        ),
        node::<u32>(
            2,
            ServiceLifetime::Transient,
            Constructor::Class(leaf),
            dependencies,
        ),
    ]);
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let owner = super::OwnerData::new(super::ROOT, sender.downgrade());
    let mut coordinator = super::Coordinator::new(graph, owner.clone(), receiver, 1);
    let cached = pending(coordinator.ensure_task(super::ROOT, 0, &mut vec![]));
    coordinator.settle(
        cached,
        Err(crate::ResolveError::new("cached failure".into())),
    );
    let (waiter, result) = tokio::sync::oneshot::channel();
    coordinator.accept_resolution(super::ROOT, 2, (0, waiter));
    assert!(
        result
            .await
            .unwrap()
            .err()
            .unwrap()
            .to_string()
            .contains("cached failure")
    );
    assert_eq!(coordinator.owners[&super::ROOT].active_tasks.len(), 1);
    coordinator.launch_ready();
    let completion = coordinator.jobs.join_next_with_id().await.unwrap();
    coordinator.handle_completion(completion);
    assert!(coordinator.owners[&super::ROOT].active_tasks.is_empty());
    // 失败共享任务已经退役，但失败缓存仍保留；被接受的 Transient 子任务正常发布。
    assert!(coordinator.tasks.is_empty());
    assert!(coordinator.owners[&super::ROOT].active_tasks.is_empty());
    assert!(matches!(
        coordinator.owners[&super::ROOT].cache.get(&0),
        Some(super::CacheEntry::Failed(_))
    ));
    assert_eq!(owner.journal.lock().unwrap().len(), 1);
    coordinator.begin_close(owner.clone(), None);
    coordinator.run().await;
    assert_eq!(owner.status.load(Ordering::Acquire), CLOSED);
    assert!(owner.journal.lock().unwrap().is_empty());
}

static CHAIN_DROPS: AtomicUsize = AtomicUsize::new(0);

struct Chain {
    depth: usize,
}

impl Drop for Chain {
    fn drop(&mut self) {
        CHAIN_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

fn chain_leaf(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(Chain { depth: 1 }))
}

fn chain_link(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    let previous = inputs.take::<Chain>(InputSlot::new(0))?;
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(Chain {
        depth: previous.depth + 1,
    }))
}

#[test]
fn deep_graph_activation_and_shutdown_do_not_use_a_recursive_rust_stack() {
    const COUNT: usize = 12_000;
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let executor = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            executor.block_on(async {
                CHAIN_DROPS.store(0, Ordering::SeqCst);
                let nodes = (0..COUNT)
                    .map(|index| {
                        let (constructor, dependencies) = if index == 0 {
                            (
                                chain_leaf as crate::activation::ClassConstructor,
                                Vec::new(),
                            )
                        } else {
                            (
                                chain_link as crate::activation::ClassConstructor,
                                vec![CompiledDependency {
                                    slot: InputSlot::new(0),
                                    requested: ServiceIdentifier::new(
                                        Some(ServiceKey::Indexed(index - 1)),
                                        ServiceType::create::<Chain>(),
                                    ),
                                    optional: false,
                                    input: DependencyInput::Immediate {
                                        target: index - 1,
                                        project: project_required::<Chain>,
                                    },
                                    label: Some("previous"),
                                }],
                            )
                        };
                        node::<Chain>(
                            index,
                            ServiceLifetime::Singleton,
                            Constructor::Class(constructor),
                            dependencies,
                        )
                    })
                    .collect();
                let (runtime, owner) =
                    Runtime::start(graph(nodes), 8, crate::InitializationMode::Lazy)
                        .await
                        .unwrap();
                let lease = tokio::time::timeout(
                    Duration::from_secs(20),
                    runtime.resolve(&owner, COUNT - 1),
                )
                .await
                .expect("a deep graph should finish")
                .unwrap();
                // SAFETY: lease 在读取期间强持有经过精确类型核对的 Chain 实例。
                assert_eq!(
                    unsafe { lease.pointer::<Chain>().unwrap().as_ref().depth },
                    COUNT
                );
                tokio::time::timeout(Duration::from_secs(20), runtime.close(&owner))
                    .await
                    .expect("iterative close should finish")
                    .unwrap();
                // 逃逸的顶层 lease 在逻辑关闭后仍强持有完整依赖链。
                assert_eq!(CHAIN_DROPS.load(Ordering::SeqCst), 0);
                drop(lease);
                assert_eq!(CHAIN_DROPS.load(Ordering::SeqCst), COUNT);
            });
        })
        .unwrap()
        .join()
        .unwrap();
}

struct Gates {
    active: AtomicUsize,
    maximum: AtomicUsize,
    started: Semaphore,
    release: Semaphore,
}

static GLOBAL_GATES: OnceLock<Arc<Gates>> = OnceLock::new();
struct Gated;

fn gated(_inputs: FactoryInputs<'_>) -> FactoryFuture<'_> {
    Box::pin(async {
        let gates = GLOBAL_GATES.get().unwrap();
        let active = gates.active.fetch_add(1, Ordering::SeqCst) + 1;
        gates.maximum.fetch_max(active, Ordering::SeqCst);
        gates.started.add_permits(1);
        gates.release.acquire().await.unwrap().forget();
        gates.active.fetch_sub(1, Ordering::SeqCst);
        Ok(ErasedService::new(Gated))
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn activation_limit_is_shared_by_concurrent_gets_and_all_scopes() {
    const LIMIT: usize = 3;
    const COUNT: usize = 30;
    let gates = Arc::new(Gates {
        active: AtomicUsize::new(0),
        maximum: AtomicUsize::new(0),
        started: Semaphore::new(0),
        release: Semaphore::new(0),
    });
    assert!(GLOBAL_GATES.set(gates.clone()).is_ok());
    let (runtime, owner) = Runtime::start(
        graph(vec![node::<Gated>(
            0,
            ServiceLifetime::Transient,
            Constructor::Factory(FactoryInvoker::Async(gated)),
            Vec::new(),
        )]),
        LIMIT,
        crate::InitializationMode::Lazy,
    )
    .await
    .unwrap();
    let scope_a = runtime
        .create_scope(crate::InitializationMode::Lazy)
        .await
        .unwrap();
    let scope_b = runtime
        .create_scope(crate::InitializationMode::Lazy)
        .await
        .unwrap();
    let owners = [owner.clone(), scope_a, scope_b];
    let mut requests = Vec::new();
    for index in 0..COUNT {
        let runtime = runtime.clone();
        let owner = owners[index % owners.len()].clone();
        requests.push(tokio::spawn(
            async move { runtime.resolve(&owner, 0).await },
        ));
    }
    tokio::time::timeout(
        Duration::from_secs(3),
        gates.started.acquire_many(LIMIT as u32),
    )
    .await
    .expect("the scheduler should saturate its available concurrency")
    .unwrap()
    .forget();
    assert_eq!(gates.active.load(Ordering::SeqCst), LIMIT);
    gates.release.add_permits(COUNT);
    for request in requests {
        request.await.unwrap().unwrap();
    }
    assert_eq!(gates.maximum.load(Ordering::SeqCst), LIMIT);
    tokio::time::timeout(Duration::from_secs(3), runtime.close(&owner))
        .await
        .unwrap()
        .unwrap();
    assert!(owners.iter().all(|owner| owner.is_closed()));
}

struct LayerGates {
    slow_started: Semaphore,
    slow_release: Semaphore,
    consumer_started: Semaphore,
}

static LAYER_GATES: OnceLock<LayerGates> = OnceLock::new();
struct Fast;
struct Slow;
struct Consumer;

fn fast(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(Fast))
}

fn slow(_inputs: FactoryInputs<'_>) -> FactoryFuture<'_> {
    Box::pin(async {
        let gates = LAYER_GATES.get().unwrap();
        gates.slow_started.add_permits(1);
        gates.slow_release.acquire().await.unwrap().forget();
        Ok(ErasedService::new(Slow))
    })
}

fn fast_consumer(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    let _dependency = inputs.take::<Fast>(InputSlot::new(0))?;
    inputs.ensure_all_consumed()?;
    LAYER_GATES.get().unwrap().consumer_started.add_permits(1);
    Ok(ErasedService::new(Consumer))
}

#[tokio::test]
async fn a_ready_successor_does_not_wait_for_an_unrelated_slow_node() {
    assert!(
        LAYER_GATES
            .set(LayerGates {
                slow_started: Semaphore::new(0),
                slow_release: Semaphore::new(0),
                consumer_started: Semaphore::new(0),
            })
            .is_ok()
    );
    let graph = graph(vec![
        node::<Fast>(
            0,
            ServiceLifetime::Singleton,
            Constructor::Class(fast),
            Vec::new(),
        ),
        node::<Slow>(
            1,
            ServiceLifetime::Singleton,
            Constructor::Factory(FactoryInvoker::Async(slow)),
            Vec::new(),
        ),
        node::<Consumer>(
            2,
            ServiceLifetime::Singleton,
            Constructor::Class(fast_consumer),
            vec![CompiledDependency {
                slot: InputSlot::new(0),
                requested: ServiceIdentifier::new(
                    Some(ServiceKey::Indexed(0)),
                    ServiceType::create::<Fast>(),
                ),
                optional: false,
                input: DependencyInput::Immediate {
                    target: 0,
                    project: project_required::<Fast>,
                },
                label: Some("fast"),
            }],
        ),
    ]);
    let creating =
        tokio::spawn(
            async move { Runtime::start(graph, 2, crate::InitializationMode::Eager).await },
        );
    let gates = LAYER_GATES.get().unwrap();
    tokio::time::timeout(Duration::from_secs(3), gates.slow_started.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    tokio::time::timeout(Duration::from_secs(3), gates.consumer_started.acquire())
        .await
        .expect("ready successors must be released before unrelated work finishes")
        .unwrap()
        .forget();
    assert!(!creating.is_finished());
    gates.slow_release.add_permits(1);
    let (runtime, owner) = creating.await.unwrap().unwrap();
    runtime.close(&owner).await.unwrap();
}

#[tokio::test]
async fn closing_empty_scopes_and_root_needs_no_extra_event_to_complete() {
    let (runtime, owner) = Runtime::start(graph(Vec::new()), 1, crate::InitializationMode::Lazy)
        .await
        .unwrap();
    let mut scopes = Vec::new();
    for _ in 0..8 {
        scopes.push(
            runtime
                .create_scope(crate::InitializationMode::Lazy)
                .await
                .unwrap(),
        );
    }
    tokio::time::timeout(Duration::from_secs(1), runtime.close(&owner))
        .await
        .expect("root should close immediately after its last empty scope")
        .unwrap();
}

struct ParallelService;

struct Rendezvous {
    arrived: Mutex<usize>,
    both_arrived: Condvar,
}

static PARALLEL_RENDEZVOUS: OnceLock<Rendezvous> = OnceLock::new();

fn parallel_constructor(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    let rendezvous = PARALLEL_RENDEZVOUS.get().unwrap();
    let mut arrived = rendezvous.arrived.lock().unwrap();
    *arrived += 1;
    rendezvous.both_arrived.notify_all();
    let (arrived, _) = rendezvous
        .both_arrived
        .wait_timeout_while(arrived, Duration::from_secs(3), |arrived| *arrived != 2)
        .unwrap();
    assert_eq!(
        *arrived, 2,
        "independent synchronous constructors must run on different workers"
    );
    Ok(ErasedService::new(ParallelService))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn synchronous_ready_nodes_can_execute_in_parallel_on_tokio_workers() {
    assert!(
        PARALLEL_RENDEZVOUS
            .set(Rendezvous {
                arrived: Mutex::new(0),
                both_arrived: Condvar::new(),
            })
            .is_ok()
    );
    let nodes = (0..2)
        .map(|index| {
            node::<ParallelService>(
                index,
                ServiceLifetime::Singleton,
                Constructor::Class(parallel_constructor),
                Vec::new(),
            )
        })
        .collect();
    let (runtime, owner) = Runtime::start(graph(nodes), 2, crate::InitializationMode::Eager)
        .await
        .unwrap();
    runtime.close(&owner).await.unwrap();
}
