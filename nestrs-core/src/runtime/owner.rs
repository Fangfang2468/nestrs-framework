//! owner 的公开存活边界与协调器内部状态。
//!
//! `Owner` 随门面存活，`OwnerState` 仅由协调器修改；两者共享 `OwnerData`。
//! journal 必须放在共享数据里：即使 Tokio 已停止、协调器 future 已被丢弃，
//! 门面借用仍会保活成功发布的实例，已返回的 `&T` 才不会悬垂。

use std::{
    collections::{HashMap, HashSet},
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
    // 这是跨线程可见的“是否还接受请求”，不是协调器详细关闭阶段的第二份拷贝。
    pub(super) status: AtomicU8,
    pub(super) journal: Mutex<Vec<Published>>,
    close_result: Mutex<Option<Result<(), DisposeError>>>,
}

impl OwnerData {
    pub(super) fn new(id: OwnerId) -> Arc<Self> {
        Arc::new(Self {
            id,
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

    /// 按逆发布顺序取出一个实例。锁在返回前释放，绝不持锁调用用户 cleanup/Drop。
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
