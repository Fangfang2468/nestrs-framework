//! owner 的公开存活边界与协调器内部状态。
//!
//! `Owner` 随门面存活，`OwnerState` 仅由协调器修改；两者共享 `OwnerData`。
//! journal 必须放在共享数据里：即使 Tokio 已停止、协调器 future 已被丢弃，
//! 门面借用仍会保活成功发布的实例，已返回的 `&T` 才不会悬垂。

use ahash::{AHashMap, AHashSet};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

use tokio::sync::mpsc;

use crate::{
    activation::DependencyLease,
    error::{DisposeError, ResolveError},
};

use super::{CloseWaiter, OwnerId, Resolution, TaskId, handle::Command};

/// 每个运行时为 root 保留的 owner 编号。
pub(super) const ROOT: OwnerId = 0;

/// 允许接受新请求的共享状态值。
pub(super) const OPEN: u8 = 0;

/// 已拒绝新请求、仍在排空或清理的共享状态值。
const CLOSING: u8 = 1;

/// 关闭结果已发布的共享状态值。
pub(super) const CLOSED: u8 = 2;

/// 实际 root/scope 借用的终点。协调器只持有 data，不能反向保活这个句柄。
pub(crate) struct Owner {
    /// 门面、协调器和弱 lazy 能力共同引用的 owner 数据。
    pub(super) data: Arc<OwnerData>,

    /// 门面持有的强命令句柄，保证存活期间仍能向协调器发送请求。
    pub(super) commands: mpsc::UnboundedSender<Command>,
}

impl Owner {
    /// 读取跨线程状态；正在关闭与已关闭都拒绝新请求。
    pub(crate) fn is_closed(&self) -> bool {
        self.data.status.load(Ordering::Acquire) != OPEN
    }
}

impl Drop for Owner {
    /// 仅向已有协调器发送关闭请求，不阻塞线程或创建新运行时。
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

/// 门面与协调器共享的发布记录及关闭状态，独立保活已经返回的实例。
pub(super) struct OwnerData {
    /// 当前 root 或 scope 在本运行时中的唯一编号。
    pub(super) id: OwnerId,

    /// 一个 owner 的全部延迟字段复用此弱通道，不为每个字段分配 resolver，
    /// 也不会使 journal 中的服务反向保活运行时。
    pub(super) commands: mpsc::WeakUnboundedSender<Command>,

    /// 跨线程发布是否接受新请求，不复制协调器的详细关闭阶段。
    pub(super) status: AtomicU8,

    /// 已发布实例的强保活记录，逻辑清理按消费者优先取出。
    pub(super) journal: Mutex<Vec<Published>>,

    /// 关闭完成后保存的结果，供迟到或重复等待者复用。
    close_result: Mutex<Option<Result<(), DisposeError>>>,
}

impl OwnerData {
    /// 建立当前 owner 的初始开放状态；实例发布和关闭结果随后按协议写入。
    pub(super) fn new(id: OwnerId, commands: mpsc::WeakUnboundedSender<Command>) -> Arc<Self> {
        Arc::new(Self {
            id,
            commands,
            status: AtomicU8::new(OPEN),
            journal: Mutex::new(Vec::new()),
            close_result: Mutex::new(None),
        })
    }

    /// 发布停止接收新请求的状态，实际排空由协调器继续推进。
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

    /// 排空后使用协调器的共享工作空间重排；实例始终由当前 journal 保活。
    pub(super) fn order_cleanup(
        &self,
        graph: &crate::graph::ValidatedGraph,
        order: &mut super::cleanup::CleanupOrder,
    ) {
        let mut journal = self
            .journal
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        order.order(&mut journal, graph);
    }

    /// 取出一个已排序实例。锁在返回前释放，绝不持锁调用用户 cleanup/Drop。
    pub(super) fn next_cleanup(&self) -> Option<Published> {
        self.journal
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pop()
    }

    /// 读取已经发布的最终关闭结果，供迟到的关闭等待者复用。
    pub(super) fn completed_close(&self) -> Option<Result<(), DisposeError>> {
        self.close_result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    /// 先保存关闭结果再发布 Closed，使观察者可取得一致结果。
    pub(super) fn complete_close(&self, result: Result<(), DisposeError>) {
        // 先保存结果再发布 CLOSED，晚到的显式等待者仍能取得同一个关闭结果。
        *self
            .close_result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(result);
        self.status.store(CLOSED, Ordering::Release);
    }
}

/// owner 已接管的实例记录，保存 provider 编号以定位 cleanup 策略。
pub(super) struct Published {
    /// 冻结计划中的 provider 节点编号。
    pub(super) provider: usize,

    /// 保活已选真实实例的强所有权凭证。
    pub(super) lease: DependencyLease,
}

/// 共享生命周期的缓存独立于任务表。任务结束后立刻退役，成功/失败仍缓存到 owner 关闭。
pub(super) enum CacheEntry {
    /// 共享服务仍在构造，后续请求挂到同一任务。
    Building(TaskId),

    /// 共享服务已成功发布，复用同一个实例 lease。
    Ready(DependencyLease),

    /// 共享服务构造失败，保留错误到 owner 关闭。
    Failed(ResolveError),
}

impl CacheEntry {
    /// 将已完成的解析结果转换为 owner 的成功或失败缓存。
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
    /// 接收新的查询和构造工作。
    Open,

    /// 拒绝新查询，等待已接受任务完成。
    Draining,

    /// 逐实例执行本 owner 的 cleanup 与同步释放。
    Cleaning {
        /// 当前 owner 是否已有 cleanup worker，保证逐 owner 串行。
        running: bool,
    },

    /// 关闭已经完成且结果已写入共享数据。
    Closed,
}

/// 只有协调器可修改的 owner 缓存、活跃任务与详细关闭阶段。
pub(super) struct OwnerState {
    /// 门面、协调器和弱 lazy 能力共同引用的 owner 数据。
    pub(super) data: Arc<OwnerData>,

    /// 仅协调器修改的排空、清理与完成阶段。
    pub(super) phase: OwnerPhase,

    /// 当前 owner 的共享服务任务、成功实例或失败缓存。
    pub(super) cache: AHashMap<usize, CacheEntry>,

    /// 只含未结束的 occurrence；is_empty 就是排空条件，不另维护容易失配的计数。
    pub(super) active_tasks: AHashSet<TaskId>,

    /// 当前 owner 清理过程汇总的失败文本。
    pub(super) errors: Vec<String>,

    /// 所有等待本 owner 完整关闭的发送端。
    pub(super) close_waiters: Vec<CloseWaiter>,
}

impl OwnerState {
    /// 建立当前 owner 的初始开放状态；实例发布和关闭结果随后按协议写入。
    pub(super) fn new(data: Arc<OwnerData>) -> Self {
        Self {
            data,
            phase: OwnerPhase::Open,
            cache: AHashMap::new(),
            active_tasks: AHashSet::new(),
            errors: Vec::new(),
            close_waiters: Vec::new(),
        }
    }

    /// 仅从开放状态进入排空，重复关闭不会重启清理阶段。
    pub(super) fn begin_close(&mut self) {
        // 重复 Close 不能把正在清理的 owner 退回 Draining，否则可能重复执行 hook。
        if self.phase == OwnerPhase::Open {
            self.phase = OwnerPhase::Draining;
            self.data.mark_closing();
        }
    }
}
