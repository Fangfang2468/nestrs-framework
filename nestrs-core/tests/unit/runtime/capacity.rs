//! 容量维护不能消费运行中记录，也不能按单次低谷反复缩容。

use super::*;
use crate::runtime::{
    coordinator::Coordinator,
    owner::{CacheEntry, OwnerData, ROOT},
};

fn unused(inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    inputs.ensure_all_consumed()?;
    Ok(ErasedService::new(1_u32))
}

#[test]
fn capacity_recovery_preserves_active_tasks_queries_owners_and_failures() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let root = OwnerData::new(ROOT, sender.downgrade());
    let mut coordinator = Coordinator::new(
        graph(vec![node::<u32>(
            0,
            ServiceLifetime::Transient,
            Constructor::Class(unused),
            vec![],
        )]),
        root,
        receiver,
        32,
    );
    coordinator.tasks.reserve(8192);
    coordinator.query_tasks.reserve(8192);
    coordinator.job_kinds.reserve(8192);
    coordinator.owners.reserve(8192);
    coordinator.ready.reserve(8192);
    coordinator.closing_owners.reserve(8192);
    coordinator
        .owners
        .get_mut(&ROOT)
        .unwrap()
        .active_tasks
        .reserve(8192);
    let failure = crate::ResolveError::new("retained failure".into());
    let failure_text = failure.to_string();
    coordinator
        .owners
        .get_mut(&ROOT)
        .unwrap()
        .cache
        .insert(71, CacheEntry::Failed(failure));
    let mut expansion = Vec::new();
    let tasks: Vec<_> = (0..256)
        .map(|query| {
            let task = pending(coordinator.ensure_task(ROOT, 0, &mut expansion));
            coordinator.query_tasks.insert(query, task);
            coordinator.ready.push_back(task);
            task
        })
        .collect();
    let capacity = coordinator.tasks.capacity();
    for _ in 0..4095 {
        coordinator.maintain_capacity();
    }
    assert_eq!(
        coordinator.tasks.capacity(),
        capacity,
        "窗口完成前不能按临时低谷缩容"
    );
    coordinator.maintain_capacity();
    assert!(coordinator.tasks.capacity() < capacity);
    assert!(coordinator.owners.capacity() < 8192);
    assert!(coordinator.job_kinds.capacity() < 8192);
    assert!(coordinator.ready.capacity() < 8192);
    assert!(coordinator.closing_owners.capacity() < 8192);
    assert_eq!(coordinator.tasks.len(), tasks.len());
    assert_eq!(coordinator.owners[&ROOT].active_tasks.len(), tasks.len());
    assert_eq!(coordinator.ready.iter().copied().collect::<Vec<_>>(), tasks);
    for (query, &task) in tasks.iter().enumerate() {
        assert_eq!(coordinator.query_tasks[&(query as u64)], task);
        assert_eq!(coordinator.tasks[&task].owner, ROOT);
        assert!(coordinator.owners[&ROOT].active_tasks.contains(&task));
    }
    assert!(
        matches!(coordinator.owners[&ROOT].cache.get(&71), Some(CacheEntry::Failed(error)) if error.to_string() == failure_text)
    );
    let capacity = coordinator.tasks.capacity();
    for _ in 0..8192 {
        coordinator.maintain_capacity();
    }
    assert_eq!(
        coordinator.tasks.capacity(),
        capacity,
        "同一负载不应每窗口重新缩容"
    );
}

#[test]
fn a_recent_burst_is_retained_until_a_full_low_demand_window() {
    let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
    let root = OwnerData::new(ROOT, sender.downgrade());
    let mut coordinator = Coordinator::new(graph(vec![]), root, receiver, 32);
    coordinator.ready.extend(0..4096);
    let capacity = coordinator.ready.capacity();
    coordinator.maintain_capacity();
    coordinator.ready.clear();
    for _ in 0..4095 {
        coordinator.maintain_capacity();
    }
    assert_eq!(coordinator.ready.capacity(), capacity);
    for _ in 0..4096 {
        coordinator.maintain_capacity();
    }
    assert!(coordinator.ready.capacity() <= 128);
}
