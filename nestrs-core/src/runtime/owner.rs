//! owner 的公开存活边界与协调器内部状态。
//!
//! `Owner` 随门面存活，`OwnerState` 仅由协调器修改；两者共享 `OwnerData`。
//! journal 必须放在共享数据里：即使 Tokio 已停止、协调器 future 已被丢弃，
//! 门面借用仍会保活成功发布的实例，已返回的 `&T` 才不会悬垂。

use std::{
    collections::{BinaryHeap, HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
    },
};

use tokio::sync::mpsc;

use crate::{
    activation::DependencyLease,
    error::{DisposeError, ResolveError},
};

use super::{CloseWaiter, OwnerId, Resolution, TaskId, handle::Command};

pub(super) const ROOT: OwnerId = 0;
pub(super) const OPEN: u8 = 0;
const CLOSING: u8 = 1;
pub(super) const CLOSED: u8 = 2;

/// 实际 root/scope 借用的终点。协调器只持有 data，不能反向保活这个句柄。
pub(crate) struct Owner {
    pub(super) data: Arc<OwnerData>,
    pub(super) commands: mpsc::UnboundedSender<Command>,
}

impl Owner {
    pub(crate) fn is_closed(&self) -> bool {
        self.data.status.load(Ordering::Acquire) != OPEN
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        // 非阻塞关闭只在这一处兜底。最后一个句柄消失即可通知既有协调器；
        // 不能在 Drop 里新建 runtime/block_on，也不能把取消 dispose 的等待当成取消关闭。
        if self.data.status.load(Ordering::Acquire) != CLOSED {
            let _ = self.commands.send(Command::Close {
                owner: self.data.clone(),
                waiter: None,
            });
        }
    }
}

pub(super) struct OwnerData {
    pub(super) id: OwnerId,
    // 一个 owner 的全部延迟字段复用此弱通道，不为每个字段再分配 resolver。
    // 弱引用不会使 owner journal 中的服务反向延长整个 runtime 的逻辑存活。
    pub(super) commands: mpsc::WeakUnboundedSender<Command>,
    // 这是跨线程可见的“是否还接受请求”，不是协调器详细关闭阶段的第二份拷贝。
    pub(super) status: AtomicU8,
    pub(super) journal: Mutex<Vec<Published>>,
    close_result: Mutex<Option<Result<(), DisposeError>>>,
}

impl OwnerData {
    pub(super) fn new(id: OwnerId, commands: mpsc::WeakUnboundedSender<Command>) -> Arc<Self> {
        Arc::new(Self {
            id,
            commands,
            status: AtomicU8::new(OPEN),
            journal: Mutex::new(Vec::new()),
            close_result: Mutex::new(None),
        })
    }

    pub(super) fn mark_closing(&self) {
        self.status.store(CLOSING, Ordering::Release);
    }

    /// 先确立 owner 的强持有，再让缓存、消费者或查询等待者看见成功结果。
    pub(super) fn publish(&self, provider: usize, lease: DependencyLease) {
        self.journal
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(Published { provider, lease });
    }

    /// 排空后确定清理顺序。普通依赖的发布顺序已经正确，只有图含延迟边时才需要调整。
    ///
    /// 在完整冻结 DAG 上反向执行 Kahn：一个 provider 的所有消费者实例清理后，才能
    /// 轮到它。没有实例的节点也参加拓扑传播，避免漏掉经过未实例化节点的间接约束。
    /// 同时可选的实例按发布时间倒序取出，保留无依赖节点之间原来的清理偏好。
    /// 只重排 journal，不执行用户代码；实例 lease 全程由桶、结果或 journal 保活。
    pub(super) fn order_cleanup(&self, graph: &crate::graph::ValidatedGraph) {
        if !graph.nodes.iter().any(|node| {
            node.dependencies
                .iter()
                .any(|dependency| dependency.input.is_lazy())
        }) {
            return;
        }
        let mut journal = self
            .journal
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut buckets: Vec<Vec<(usize, Published)>> =
            (0..graph.nodes.len()).map(|_| Vec::new()).collect();
        for (sequence, entry) in std::mem::take(&mut *journal).into_iter().enumerate() {
            buckets[entry.provider].push((sequence, entry));
        }
        let mut remaining: Vec<_> = graph.dependents.iter().map(Vec::len).collect();
        let mut ready = BinaryHeap::new();
        for (provider, &count) in remaining.iter().enumerate() {
            if count == 0 {
                ready.push((
                    buckets[provider].last().map_or(usize::MAX, |entry| entry.0),
                    provider,
                ));
            }
        }
        let mut ordered = Vec::new();
        while let Some((_, provider)) = ready.pop() {
            if let Some((_, entry)) = buckets[provider].pop() {
                ordered.push(entry);
                if let Some((sequence, _)) = buckets[provider].last() {
                    ready.push((*sequence, provider));
                    continue;
                }
            }
            // 拓扑计数使用去重边，构造槽位仍保持独立；重复 Transient 注入不会减两次。
            let mut targets: Vec<_> = graph.nodes[provider]
                .dependencies
                .iter()
                .filter_map(|dependency| dependency.input.target())
                .collect();
            targets.sort_unstable();
            targets.dedup();
            for target in targets {
                remaining[target] -= 1;
                if remaining[target] == 0 {
                    ready.push((
                        buckets[target].last().map_or(usize::MAX, |entry| entry.0),
                        target,
                    ));
                }
            }
        }
        debug_assert!(
            buckets.iter().all(Vec::is_empty),
            "已验证 DAG 必须可完整排序"
        );
        // next_cleanup 从尾部取出，故此处把消费者优先的结果反转保存。
        ordered.reverse();
        *journal = ordered;
    }

    /// 取出一个已排序实例。锁在返回前释放，绝不持锁调用用户 cleanup/Drop。
    pub(super) fn next_cleanup(&self) -> Option<Published> {
        self.journal
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop()
    }

    pub(super) fn completed_close(&self) -> Option<Result<(), DisposeError>> {
        self.close_result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub(super) fn complete_close(&self, result: Result<(), DisposeError>) {
        // 先保存结果再发布 CLOSED，晚到的显式等待者仍能取得同一个关闭结果。
        *self
            .close_result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(result);
        self.status.store(CLOSED, Ordering::Release);
    }
}

pub(super) struct Published {
    pub(super) provider: usize,
    pub(super) lease: DependencyLease,
}

/// 共享生命周期的缓存独立于任务表。任务结束后立刻退役，成功/失败仍缓存到 owner 关闭。
pub(super) enum CacheEntry {
    Building(TaskId),
    Ready(DependencyLease),
    Failed(ResolveError),
}

impl CacheEntry {
    pub(super) fn completed(result: &Resolution) -> Self {
        match result {
            Ok(lease) => Self::Ready(lease.clone()),
            Err(error) => Self::Failed(error.clone()),
        }
    }
}

/// 协调器唯一拥有的关闭阶段。
///
/// Open 接受工作；Draining 拒绝新查询但继续排空已接受的任务；Cleaning 逐实例清理；
/// Closed 表示结果已写入共享数据。root 进入 Cleaning 前还须等待全部 scope 移除。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum OwnerPhase {
    Open,
    Draining,
    Cleaning { running: bool },
    Closed,
}

pub(super) struct OwnerState {
    pub(super) data: Arc<OwnerData>,
    pub(super) phase: OwnerPhase,
    pub(super) cache: HashMap<usize, CacheEntry>,
    // 集合只含未结束的 occurrence；is_empty 就是排空条件，不另维护容易失配的计数。
    pub(super) active_tasks: HashSet<TaskId>,
    pub(super) errors: Vec<String>,
    pub(super) close_waiters: Vec<CloseWaiter>,
}

impl OwnerState {
    pub(super) fn new(data: Arc<OwnerData>) -> Self {
        Self {
            data,
            phase: OwnerPhase::Open,
            cache: HashMap::new(),
            active_tasks: HashSet::new(),
            errors: Vec::new(),
            close_waiters: Vec::new(),
        }
    }

    pub(super) fn begin_close(&mut self) {
        // 重复 Close 不能把正在清理的 owner 退回 Draining，否则可能重复执行 hook。
        if self.phase == OwnerPhase::Open {
            self.phase = OwnerPhase::Draining;
            self.data.mark_closing();
        }
    }
}
