use std::{
    collections::HashMap,
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use tokio::sync::Semaphore;

use super::Runtime;
use crate::{
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, FactoryFuture, FactoryInputs,
        InputSlot, prepare_required,
    },
    graph::{CompiledDependency, CompiledNode, Constructor, ValidatedGraph},
    lifetime::ServiceLifetime,
    registration::provider::{FactoryInvoker, ProviderCommon},
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};

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
        common: ProviderCommon {
            lifetime,
            primary: false,
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
            if let Some(provider) = dependency.target {
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
        routes: HashMap::new(),
    })
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
                                    target: Some(index - 1),
                                    prepare: prepare_required::<Chain>,
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
                let (runtime, owner) = Runtime::start(graph(nodes), 8);
                let lease = tokio::time::timeout(
                    Duration::from_secs(20),
                    runtime.resolve(&owner, COUNT - 1),
                )
                .await
                .expect("a deep graph should finish")
                .unwrap();
                // SAFETY: lease retains this exact type for the duration of the read.
                assert_eq!(
                    unsafe { lease.pointer::<Chain>().unwrap().as_ref().depth },
                    COUNT
                );
                tokio::time::timeout(Duration::from_secs(20), runtime.close(&owner))
                    .await
                    .expect("iterative close should finish")
                    .unwrap();
                // An escaped top lease retains the entire graph beyond logical close.
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
    );
    let scope_a = runtime.create_scope();
    let scope_b = runtime.create_scope();
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
                target: Some(0),
                prepare: prepare_required::<Fast>,
                label: Some("fast"),
            }],
        ),
    ]);
    let (runtime, owner) = Runtime::start(graph, 2);
    let warming = {
        let runtime = runtime.clone();
        let owner = owner.clone();
        tokio::spawn(async move { runtime.warm_up(&owner, ServiceLifetime::Singleton).await })
    };
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
    assert!(!warming.is_finished());
    gates.slow_release.add_permits(1);
    warming.await.unwrap().unwrap();
    runtime.close(&owner).await.unwrap();
}

#[tokio::test]
async fn closing_empty_scopes_and_root_needs_no_extra_event_to_complete() {
    let (runtime, owner) = Runtime::start(graph(Vec::new()), 1);
    let _scopes: Vec<_> = (0..8).map(|_| runtime.create_scope()).collect();
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
    let (runtime, owner) = Runtime::start(graph(nodes), 2);
    runtime
        .warm_up(&owner, ServiceLifetime::Singleton)
        .await
        .unwrap();
    runtime.close(&owner).await.unwrap();
}
