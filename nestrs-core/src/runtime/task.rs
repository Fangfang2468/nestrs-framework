//! 一次服务消费所需的活跃构造任务。
//!
//! provider 是静态声明，task 是某次实际构造 occurrence。Singleton/Scoped 可共享 task，
//! Transient 每个消费槽位创建新的 task；不要把重复依赖槽位合并为同一个 Transient。

use crate::activation::DependencyLease;

use super::{OwnerId, Resolution, ResolveWaiter, TaskId};

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
    pub(super) parents: Vec<(TaskId, usize)>,
    pub(super) waiters: Vec<ResolveWaiter>,
}
