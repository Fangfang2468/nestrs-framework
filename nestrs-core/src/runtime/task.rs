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
    /// 解析仍等待任务完成，返回活跃任务编号。
    Pending(TaskId),

    /// 已有完成结果，可直接向当前消费者交付。
    Cached(Resolution),
}

/// 输入只存在于等待/就绪阶段，worker 启动后拥有这些 lease。
///
/// 未展开单独成态，避免“依赖计数为零”被误认为已经就绪。完成态不放在这里：
/// 完成后任务移出表，共享服务的结果进入 owner 缓存，Transient 的结果交给消费者与 journal。
pub(super) enum TaskState {
    /// 普通输入尚未展开，不能仅凭计数为零判定就绪。
    Unexpanded,

    /// 已登记子任务，等待剩余普通输入完成。
    Waiting {
        /// 按输入槽位保存已就绪的普通依赖 lease。
        inputs: Vec<Option<DependencyLease>>,

        /// 按输入槽位记录待完成子任务；消费者提前失败时据此直接注销反向边，
        /// 不扫描其他任务，也不取消仍须排空的子任务。
        children: WaitingChildren,

        /// 尚未就绪的普通输入数量，归零后才可进入就绪队列。
        remaining: usize,
    },

    /// 普通输入全部就绪，正在等待构造名额。
    Queued {
        /// 按输入槽位保存已就绪的普通依赖 lease。
        inputs: Vec<Option<DependencyLease>>,
    },

    /// 输入已交给 worker，正在执行业务构造。
    Running,
}

/// 一次真实构造的 owner、输入状态及所有尚待交付的订阅。
pub(super) struct Activation {
    /// 本次操作所属的实际 root 或 scope。
    pub(super) owner: OwnerId,

    /// 冻结计划中的 provider 节点编号。
    pub(super) provider: usize,

    /// 该 occurrence 当前的输入展开与构造阶段。
    pub(super) state: TaskState,

    /// 等待本任务的消费者及其原输入槽位；同一消费者的重复注入也逐槽保留。
    pub(super) parents: CompactSet<(TaskId, usize)>,

    /// 按查询编号保存可独立取消的普通等待者。
    pub(super) query_waiters: CompactMap<QueryId, ResolveWaiter>,

    /// 延迟接收端归字段所有，取消一次 get 不注销仍可接续的 watch 发送端。
    pub(super) lazy_waiters: CompactList<tokio::sync::watch::Sender<Option<Resolution>>>,
}

/// 只记录真实 Pending 的输入。缓存命中、缺席和 Lazy 输入不分配子任务槽位；
/// 首个子任务直接内联，多个子任务才使用按原输入编号索引的数组。
pub(super) enum WaitingChildren {
    /// 没有仍需等待的子任务。
    Empty,

    /// 一个待完成子任务直接内联保存。
    One {
        /// 消费者原始输入槽位编号。
        input: usize,

        /// 该槽位等待的真实子任务编号。
        task: TaskId,
    },

    /// 多个待完成子任务按原输入编号保存在数组中。
    Many(Box<[Option<TaskId>]>),
}

impl WaitingChildren {
    /// 按原输入槽位登记待完成子任务，保留重复类型的独立消费关系。
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

    /// 移走指定输入的子任务记录，供完成或失败退订使用。
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

    /// 只枚举仍在等待的真实输入及子任务编号。
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
    /// 带唯一编号、允许及时退订的普通查询订阅。
    Query(QueryId, ResolveWaiter),

    /// 由延迟字段持有接收端的可接续订阅。
    Lazy(tokio::sync::watch::Sender<Option<Resolution>>),
}

impl From<(QueryId, ResolveWaiter)> for ResolutionWaiter {
    /// 把普通查询编号及其一次性发送端组合为统一等待者。
    fn from((query, waiter): (QueryId, ResolveWaiter)) -> Self {
        Self::Query(query, waiter)
    }
}

impl ResolutionWaiter {
    /// 按订阅类型交付一次结果；接收方取消不影响任务本身。
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
