//! 一次服务消费所需的活跃构造任务。
//!
//! provider 是静态声明，task 是某次实际构造 occurrence。Singleton/Scoped 可共享 task，
//! Transient 每个消费槽位创建新的 task；不要把重复依赖槽位合并为同一个 Transient。

use crate::activation::DependencyLease;

use super::{
    OwnerId, QueryId, Resolution, ResolveWaiter, TaskId,
    compact::{CompactList, CompactMap, CompactSet},
};

/// 查缓存或创建任务的结果。只有 Pending 对应活跃任务表中的条目。
pub(super) enum TaskRequest {
    Pending(TaskId),
    Cached(Resolution),
}

/// 输入只存在于等待/就绪阶段，worker 启动后拥有这些 lease。
///
/// 未展开单独成态，避免“依赖计数为零”被误认为已经就绪。完成态不放在这里：
/// 完成后任务移出表，共享服务的结果进入 owner 缓存，Transient 的结果交给消费者与 journal。
pub(super) enum TaskState {
    Unexpanded,
    Waiting {
        inputs: Vec<Option<DependencyLease>>,
        // 按输入槽位保存仍在等待的子任务。消费者提前失败时据此直接注销反向边，
        // 不扫描其他任务，也不取消仍须排空的子任务。
        children: WaitingChildren,
        remaining: usize,
    },
    Queued {
        inputs: Vec<Option<DependencyLease>>,
    },
    Running,
}

pub(super) struct Activation {
    pub(super) owner: OwnerId,
    pub(super) provider: usize,
    pub(super) state: TaskState,
    // 同一个消费者可出现多个不同输入槽位，逐一保留才能维持重复注入语义。
    pub(super) parents: CompactSet<(TaskId, usize)>,
    pub(super) query_waiters: CompactMap<QueryId, ResolveWaiter>,
    // 延迟接收端归字段所有，取消一次 get 不应注销仍可接续的 watch 订阅。
    pub(super) lazy_waiters: CompactList<tokio::sync::watch::Sender<Option<Resolution>>>,
}

/// 只记录真实 Pending 的输入。缓存命中、缺席和 Lazy 输入不分配子任务槽位；
/// 首个子任务直接内联，多个子任务才使用按原输入编号索引的数组。
pub(super) enum WaitingChildren {
    Empty,
    One { input: usize, task: TaskId },
    Many(Box<[Option<TaskId>]>),
}

impl WaitingChildren {
    pub(super) fn insert(&mut self, input: usize, task: TaskId, slots: usize) {
        match self {
            Self::Empty => *self = Self::One { input, task },
            Self::One {
                input: previous,
                task: previous_task,
            } => {
                debug_assert_ne!(*previous, input, "每个输入只能订阅一次");
                let mut children = vec![None; slots].into_boxed_slice();
                children[*previous] = Some(*previous_task);
                children[input] = Some(task);
                *self = Self::Many(children);
            }
            Self::Many(children) => {
                debug_assert!(children[input].is_none(), "每个输入只能订阅一次");
                children[input] = Some(task);
            }
        }
    }

    pub(super) fn take(&mut self, input: usize) -> Option<TaskId> {
        match self {
            Self::Empty => None,
            Self::One {
                input: previous, ..
            } if *previous != input => None,
            Self::One { task, .. } => {
                let task = *task;
                *self = Self::Empty;
                Some(task)
            }
            Self::Many(children) => children[input].take(),
        }
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (usize, TaskId)> + '_ {
        let (one, many): (_, &[Option<TaskId>]) = match self {
            Self::Empty => (None, &[]),
            Self::One { input, task } => (Some((*input, *task)), &[]),
            Self::Many(children) => (None, children),
        };
        one.into_iter().chain(
            many.iter()
                .enumerate()
                .filter_map(|(input, &task)| task.map(|task| (input, task))),
        )
    }
}

/// 一次性查询与延迟槽位使用同一个任务完成协议。延迟槽位的接收端由句柄自身保留，
/// 因而调用者取消等待时，协调器仍能交付并缓存已经接受的 occurrence。
pub(super) enum ResolutionWaiter {
    Query(QueryId, ResolveWaiter),
    Lazy(tokio::sync::watch::Sender<Option<Resolution>>),
}

impl From<(QueryId, ResolveWaiter)> for ResolutionWaiter {
    fn from((query, waiter): (QueryId, ResolveWaiter)) -> Self {
        Self::Query(query, waiter)
    }
}

impl ResolutionWaiter {
    pub(super) fn send(self, result: Resolution) {
        match self {
            Self::Query(_, waiter) => {
                let _ = waiter.send(result);
            }
            Self::Lazy(waiter) => {
                let _ = waiter.send(Some(result));
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/task.rs"]
mod tests;
