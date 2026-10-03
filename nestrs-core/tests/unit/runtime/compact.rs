use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use ahash::{AHashMap, AHashSet};

use super::{CompactList, CompactMap, CompactSet};

struct DropProbe(usize, Arc<AtomicUsize>);

impl Drop for DropProbe {
    fn drop(&mut self) {
        self.1.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn map_replacement_cancellation_and_partial_drain_release_every_value_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut values = CompactMap::new();
    assert!(values.remove(&7).is_none());
    assert!(values.insert(7, DropProbe(0, drops.clone())).is_none());
    assert_eq!(values.remove(&8).map(|probe| probe.0), None);
    assert_eq!(values.insert(7, DropProbe(1, drops.clone())).unwrap().0, 0);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(values.remove(&7).unwrap().0, 1);
    for key in 0..4096 {
        assert!(values.insert(key, DropProbe(key, drops.clone())).is_none());
    }
    for key in (0..4096).step_by(2) {
        assert_eq!(values.remove(&key).unwrap().0, key);
        assert!(values.remove(&key).is_none());
    }
    assert_eq!(values.len(), 2048);
    let mut remaining = values.into_iter();
    drop(remaining.next());
    drop(remaining);
    assert_eq!(drops.load(Ordering::Relaxed), 4098);
}

#[test]
fn parent_set_preserves_distinct_input_slots_and_large_fan_out() {
    let mut parents = CompactSet::new();
    assert!(parents.insert((10_u64, 0_usize)));
    assert!(!parents.insert((10, 0)));
    assert!(!parents.remove(&(10, 1)));
    assert!(parents.insert((10, 1)));
    for task in 11..4096 {
        assert!(parents.insert((task, 0)));
        assert!(parents.insert((task, 1)));
    }
    for task in 10..4096 {
        assert!(parents.remove(&(task, 0)));
        assert!(parents.contains(&(task, 1)));
    }
    assert_eq!(parents.len(), 4086);
    assert!(parents.into_iter().all(|(_, slot)| slot == 1));
}

#[test]
fn lazy_subscribers_keep_order_and_drop_after_partial_delivery() {
    for count in [0, 1, 2, 128] {
        let drops = Arc::new(AtomicUsize::new(0));
        let mut values = CompactList::new();
        for index in 0..count {
            values.push(DropProbe(index, drops.clone()));
        }
        assert_eq!(values.len(), count);
        let mut values = values.into_iter();
        for index in 0..count.min(3) {
            assert_eq!(values.next().unwrap().0, index);
        }
        drop(values);
        assert_eq!(drops.load(Ordering::Relaxed), count);
    }
}

#[test]
fn compact_subscription_records_reduce_inline_task_storage() {
    use crate::{
        activation::DependencyLease,
        runtime::{OwnerId, QueryId, Resolution, ResolveWaiter, TaskId},
    };

    type Parent = (TaskId, usize);
    type LazyWaiter = tokio::sync::watch::Sender<Option<Resolution>>;
    // 同一目标上的旧字段布局，仅用于记录此次压缩收益，不承担运行时行为。
    #[allow(dead_code)]
    enum PreviousTaskState {
        Unexpanded,
        Waiting {
            inputs: Vec<Option<DependencyLease>>,
            children: Vec<Option<TaskId>>,
            remaining: usize,
        },
        Queued {
            inputs: Vec<Option<DependencyLease>>,
        },
        Running,
    }
    #[allow(dead_code)]
    struct PreviousActivation {
        owner: OwnerId,
        provider: usize,
        state: PreviousTaskState,
        parents: AHashSet<Parent>,
        query_waiters: AHashMap<QueryId, ResolveWaiter>,
        lazy_waiters: Vec<LazyWaiter>,
    }
    let sizes = [
        (
            "parents",
            size_of::<AHashSet<Parent>>(),
            size_of::<CompactSet<Parent>>(),
        ),
        (
            "query_waiters",
            size_of::<AHashMap<QueryId, ResolveWaiter>>(),
            size_of::<CompactMap<QueryId, ResolveWaiter>>(),
        ),
        (
            "lazy_waiters",
            size_of::<Vec<LazyWaiter>>(),
            size_of::<CompactList<LazyWaiter>>(),
        ),
    ];
    for (name, old, new) in sizes {
        println!("{name}: {old} -> {new} bytes");
        assert!(
            new < old,
            "{name} must keep the single-subscriber path compact"
        );
    }
    println!(
        "Activation: {} -> {} bytes; TaskState: {} -> {} bytes",
        size_of::<PreviousActivation>(),
        size_of::<crate::runtime::task::Activation>(),
        size_of::<PreviousTaskState>(),
        size_of::<crate::runtime::task::TaskState>()
    );
    assert!(size_of::<crate::runtime::task::Activation>() < size_of::<PreviousActivation>());
}
