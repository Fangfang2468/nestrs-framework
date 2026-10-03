//! 未完成的共享构造只保留仍有效的订阅；取消查询或父任务失败不取消其构造。

use ahash::AHashMap;
use std::sync::Arc;

use tokio::sync::{mpsc, oneshot, watch};

use super::{graph, node};
use crate::{
    ResolveError, ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, DependencyLease, ErasedService, InputSlot,
        project_required,
    },
    graph::{CompiledDependency, Constructor, DependencyInput, ValidatedGraph},
    runtime::{
        TaskId,
        coordinator::Coordinator,
        handle::Command,
        owner::{CacheEntry, OwnerData, ROOT},
        task::TaskState,
    },
    service::{ServiceIdentifier, ServiceKey, ServiceType},
};

fn value(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(21_u32))
}

fn sum(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    let first = inputs.take::<u32>(InputSlot::new(0))?;
    let second = inputs.take::<u32>(InputSlot::new(1))?;
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(*first + *second))
}

fn dependency(input: usize, provider: usize) -> CompiledDependency {
    CompiledDependency {
        slot: InputSlot::new(input),
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
    }
}

fn fixture(
    graph: Arc<ValidatedGraph>,
) -> (Coordinator, Arc<OwnerData>, mpsc::UnboundedSender<Command>) {
    let (commands, receiver) = mpsc::unbounded_channel();
    let root = OwnerData::new(ROOT, commands.downgrade());
    (
        Coordinator::new(graph, root.clone(), receiver, 4),
        root,
        commands,
    )
}

fn building(coordinator: &Coordinator, owner: u64, provider: usize) -> TaskId {
    let CacheEntry::Building(task) = coordinator.owners[&owner].cache[&provider] else {
        panic!("provider 必须仍在构造");
    };
    task
}

fn completed_value(coordinator: &Coordinator) -> DependencyLease {
    DependencyLease::new(
        ErasedService::new(21_u32),
        vec![],
        coordinator.domain.clone(),
    )
}

#[tokio::test]
async fn cancelled_queries_retire_while_the_shared_activation_is_still_pending() {
    for lifetime in [ServiceLifetime::Singleton, ServiceLifetime::Scoped] {
        let (mut coordinator, root, commands) = fixture(graph(vec![node::<u32>(
            0,
            lifetime,
            Constructor::Class(value),
            vec![],
        )]));
        let scope = OwnerData::new(1, commands.downgrade());
        coordinator.handle_command(Command::Register(scope));
        let (waiter, survivor) = oneshot::channel();
        coordinator.accept_resolution(1, 0, (0, waiter));
        let owner = if lifetime == ServiceLifetime::Singleton {
            ROOT
        } else {
            1
        };
        let task = building(&coordinator, owner, 0);
        for query in 1..=10_000 {
            let (waiter, receiver) = oneshot::channel();
            coordinator.accept_resolution(1, 0, (query, waiter));
            assert_eq!(coordinator.tasks[&task].query_waiters.len(), 2);
            drop(receiver);
            coordinator.handle_command(Command::CancelQuery(query));
            // 先检查未完成时的状态，不能仅凭构造最后完成后全部清空判断修复有效。
            assert_eq!(coordinator.query_tasks.len(), 1);
            assert_eq!(coordinator.tasks[&task].query_waiters.len(), 1);
            assert_eq!(coordinator.tasks.len(), 1);
            assert_eq!(coordinator.owners[&owner].active_tasks.len(), 1);
        }
        let lease = completed_value(&coordinator);
        coordinator.settle(task, Ok(lease));
        let lease = survivor.await.unwrap().unwrap();
        assert!(coordinator.query_tasks.is_empty());
        assert!(coordinator.tasks.is_empty());
        assert_eq!(
            coordinator.owners[&owner]
                .data
                .journal
                .lock()
                .unwrap()
                .len(),
            1
        );
        drop(lease);
        coordinator.begin_close(root, None);
        coordinator.run().await;
    }
}

#[tokio::test]
async fn query_cancellation_and_completion_orders_preserve_other_waiters_and_publication() {
    // 0：接受前取消；1：接受后、完成前取消；2：结果已发送但调用者尚未接收时取消。
    for order in 0..3 {
        let (mut coordinator, root, _commands) = fixture(graph(vec![node::<u32>(
            0,
            ServiceLifetime::Singleton,
            Constructor::Class(value),
            vec![],
        )]));
        let (waiter, mut receiver) = oneshot::channel();
        if order == 0 {
            receiver.close();
        }
        coordinator.handle_command(Command::Resolve {
            query: 0,
            owner: ROOT,
            provider: 0,
            waiter,
        });
        let task = building(&coordinator, ROOT, 0);
        let (waiter, survivor) = oneshot::channel();
        coordinator.accept_resolution(ROOT, 0, (1, waiter));
        let (waiter, lazy) = watch::channel(None);
        coordinator.handle_command(Command::ResolveLazy {
            owner: ROOT,
            provider: 0,
            waiter,
        });
        if order != 2 {
            drop(receiver);
            coordinator.handle_command(Command::CancelQuery(0));
            assert_eq!(coordinator.tasks[&task].query_waiters.len(), 1);
            assert_eq!(coordinator.tasks[&task].lazy_waiters.len(), 1);
        }
        coordinator.settle(task, Ok(completed_value(&coordinator)));
        coordinator.handle_command(Command::CancelQuery(0));
        coordinator.handle_command(Command::CancelQuery(0));
        assert!(coordinator.query_tasks.is_empty());
        assert!(coordinator.tasks.is_empty());
        let lease = survivor.await.unwrap().unwrap();
        assert!(
            lazy.borrow()
                .as_ref()
                .unwrap()
                .as_ref()
                .unwrap()
                .ptr_eq(&lease)
        );
        assert_eq!(root.journal.lock().unwrap().len(), 1);
        drop(lease);
        drop(lazy);
        coordinator.begin_close(root, None);
        coordinator.run().await;
    }
}

#[tokio::test]
async fn cached_failure_does_not_accumulate_parent_edges_in_a_pending_shared_child() {
    let (mut coordinator, root, _commands) = fixture(graph(vec![
        node::<u32>(
            0,
            ServiceLifetime::Singleton,
            Constructor::Class(value),
            vec![],
        ),
        node::<u32>(
            1,
            ServiceLifetime::Singleton,
            Constructor::Class(value),
            vec![],
        ),
        node::<u32>(
            2,
            ServiceLifetime::Transient,
            Constructor::Class(value),
            vec![dependency(0, 0), dependency(1, 0), dependency(2, 1)],
        ),
    ]));
    coordinator.owners.get_mut(&ROOT).unwrap().cache.insert(
        1,
        CacheEntry::Failed(ResolveError::new("cached failure".into())),
    );
    for query in 0..10_000 {
        let (waiter, result) = oneshot::channel();
        coordinator.accept_resolution(ROOT, 2, (query, waiter));
        assert!(result.await.unwrap().is_err());
        let child = building(&coordinator, ROOT, 0);
        assert_eq!(coordinator.tasks.len(), 1);
        assert!(coordinator.tasks[&child].parents.is_empty());
        assert!(coordinator.query_tasks.is_empty());
        assert_eq!(coordinator.owners[&ROOT].active_tasks.len(), 1);
    }
    coordinator.launch_ready();
    let completion = coordinator.jobs.join_next_with_id().await.unwrap();
    coordinator.handle_completion(completion);
    assert_eq!(
        root.journal.lock().unwrap().len(),
        1,
        "无人等待的已接受子任务仍须发布"
    );
    coordinator.begin_close(root, None);
    coordinator.run().await;
}

#[tokio::test]
async fn failing_parents_unsubscribe_each_slot_without_removing_a_healthy_consumer() {
    let (mut coordinator, root, _commands) = fixture(graph(vec![
        node::<u32>(
            0,
            ServiceLifetime::Singleton,
            Constructor::Class(value),
            vec![],
        ),
        node::<u32>(
            1,
            ServiceLifetime::Singleton,
            Constructor::Class(value),
            vec![],
        ),
        node::<u32>(
            2,
            ServiceLifetime::Transient,
            Constructor::Class(value),
            vec![dependency(0, 0), dependency(1, 0), dependency(2, 1)],
        ),
        node::<u32>(
            3,
            ServiceLifetime::Singleton,
            Constructor::Class(sum),
            vec![dependency(0, 0), dependency(1, 0)],
        ),
    ]));
    let mut failures = Vec::new();
    for query in 0..2 {
        let (waiter, result) = oneshot::channel();
        coordinator.accept_resolution(ROOT, 2, (query, waiter));
        failures.push(result);
    }
    let (waiter, healthy) = oneshot::channel();
    coordinator.accept_resolution(ROOT, 3, (2, waiter));
    let child = building(&coordinator, ROOT, 0);
    let failing = building(&coordinator, ROOT, 1);
    let parent = building(&coordinator, ROOT, 3);
    assert_eq!(coordinator.tasks[&child].parents.len(), 6);
    coordinator.settle(failing, Err(ResolveError::new("dependency failed".into())));
    for failure in failures {
        assert!(failure.await.unwrap().is_err());
    }
    assert_eq!(coordinator.tasks[&child].parents.len(), 2);
    assert!(coordinator.tasks[&child].parents.contains(&(parent, 0)));
    assert!(coordinator.tasks[&child].parents.contains(&(parent, 1)));
    coordinator.settle(child, Ok(completed_value(&coordinator)));
    let TaskState::Queued { inputs } = &coordinator.tasks[&parent].state else {
        panic!("健康父节点应已就绪")
    };
    assert_eq!(inputs.len(), 2);
    assert!(inputs.iter().all(Option::is_some));
    coordinator.launch_ready();
    let completion = coordinator.jobs.join_next_with_id().await.unwrap();
    coordinator.handle_completion(completion);
    assert!(healthy.await.unwrap().is_ok());
    assert!(coordinator.query_tasks.is_empty());
    coordinator.begin_close(root, None);
    coordinator.run().await;
}

#[test]
fn deep_failure_unlinks_pending_siblings_without_recursive_retirement() {
    const DEPTH: usize = 12_000;
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(|| {
            let mut nodes = vec![
                node::<u32>(
                    0,
                    ServiceLifetime::Singleton,
                    Constructor::Class(value),
                    vec![],
                ),
                node::<u32>(
                    1,
                    ServiceLifetime::Singleton,
                    Constructor::Class(value),
                    vec![],
                ),
            ];
            nodes.extend((2..DEPTH + 2).map(|provider| {
                node::<u32>(
                    provider,
                    ServiceLifetime::Transient,
                    Constructor::Class(sum),
                    vec![dependency(0, provider - 1), dependency(1, 0)],
                )
            }));
            let (mut coordinator, root, _commands) = fixture(graph(nodes));
            let (waiter, mut result) = oneshot::channel();
            coordinator.accept_resolution(ROOT, DEPTH + 1, (0, waiter));
            let pending = building(&coordinator, ROOT, 0);
            let failing = building(&coordinator, ROOT, 1);
            assert_eq!(coordinator.tasks[&pending].parents.len(), DEPTH);
            coordinator.settle(failing, Err(ResolveError::new("leaf failed".into())));
            assert!(result.try_recv().unwrap().is_err());
            assert_eq!(coordinator.tasks.len(), 1);
            assert!(coordinator.tasks[&pending].parents.is_empty());
            assert_eq!(coordinator.query_tasks, AHashMap::new());
            assert_eq!(coordinator.owners[&ROOT].active_tasks.len(), 1);
            assert!(root.journal.lock().unwrap().is_empty());
        })
        .unwrap()
        .join()
        .unwrap();
}
