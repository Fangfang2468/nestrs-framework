//! 门面与中央协调器之间的异步命令通道。
//!
//! 这里负责提交、等待与句柄生命周期，不构造服务、不推进任务图。
//! 命令队列与等待端分离，使取消等待、显式关闭取消及普通 Drop 都遵守同一套关闭协议。

use super::{
    CloseWaiter, OwnerId, Resolution, ResolveWaiter,
    coordinator::Coordinator,
    owner::{Owner, OwnerData, ROOT},
};
use crate::{
    error::{DisposeError, ResolveError},
    graph::ValidatedGraph,
    lifetime::ServiceLifetime,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::{mpsc, oneshot, watch};

/// 面向门面的命令句柄；不持有可变调度状态。
///
/// 每个查询只拥有自己的 oneshot 等待端。命令一旦进入协调器，初始化便属于 owner；
/// 丢弃查询 future 不会取消构造。所有 owner 共用同一个命令通道与并发上限。
pub(crate) struct Runtime {
    graph: Arc<ValidatedGraph>,
    commands: mpsc::UnboundedSender<Command>,
    next_owner: AtomicU64,
}

impl Runtime {
    pub(crate) fn start(graph: Arc<ValidatedGraph>, max: usize) -> (Arc<Self>, Arc<Owner>) {
        assert!(max > 0, "activation concurrency must be nonzero");
        let (commands, receiver) = mpsc::unbounded_channel();
        let root = OwnerData::new(ROOT);
        let owner = Arc::new(Owner {
            data: root.clone(),
            commands: commands.clone(),
        });
        let runtime = Arc::new(Self {
            graph: graph.clone(),
            commands,
            next_owner: AtomicU64::new(1),
        });
        tokio::spawn(
            Coordinator::new(graph, root, receiver, runtime.commands.downgrade(), max).run(),
        );
        (runtime, owner)
    }

    pub(crate) fn create_scope(&self) -> Arc<Owner> {
        let data = OwnerData::new(self.next_owner.fetch_add(1, Ordering::Relaxed));
        if self.commands.send(Command::Register(data.clone())).is_err() {
            data.complete_close(Err(coordinator_stopped()));
        }
        Arc::new(Owner {
            data,
            commands: self.commands.clone(),
        })
    }

    pub(crate) async fn resolve(&self, owner: &Owner, provider: usize) -> Resolution {
        let receiver = self.request_resolution(owner, provider)?;
        receiver
            .await
            .unwrap_or_else(|_| Err(ResolveError::closed()))
    }

    fn request_resolution(
        &self,
        owner: &Owner,
        provider: usize,
    ) -> Result<oneshot::Receiver<Resolution>, ResolveError> {
        if owner.is_closed() {
            return Err(ResolveError::closed());
        }
        let (waiter, receiver) = oneshot::channel();
        self.commands
            .send(Command::Resolve {
                owner: owner.data.id,
                provider,
                waiter,
            })
            .map_err(|_| ResolveError::closed())?;
        Ok(receiver)
    }

    pub(crate) async fn warm_up(
        &self,
        owner: &Owner,
        lifetime: ServiceLifetime,
    ) -> Result<(), ResolveError> {
        if owner.is_closed() {
            return Err(ResolveError::closed());
        }
        // 先提交全部预热请求再等待，独立分支因此能同时启动；单个失败不提前放弃其他已接受任务。
        let mut receivers = Vec::new();
        for &provider in &self.graph.topological_order {
            let node = &self.graph.nodes[provider];
            if node.common.lifetime == lifetime {
                receivers.push(self.request_resolution(owner, provider)?);
            }
        }
        let mut first_error = None;
        for receiver in receivers {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err(ResolveError::closed()));
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    pub(crate) async fn close(&self, owner: &Owner) -> Result<(), DisposeError> {
        if let Some(result) = owner.data.completed_close() {
            return result;
        }
        let (waiter, receiver) = oneshot::channel();
        if self
            .commands
            .send(Command::Close {
                owner: owner.data.clone(),
                waiter: Some(waiter),
            })
            .is_err()
        {
            return owner
                .data
                .completed_close()
                .unwrap_or_else(|| Err(coordinator_stopped()));
        }
        receiver
            .await
            .unwrap_or_else(|_| Err(coordinator_stopped()))
    }
}

pub(super) fn coordinator_stopped() -> DisposeError {
    DisposeError::new(vec![
        "Tokio 协调任务已经停止；异步 cleanup 未确认完成".to_owned(),
    ])
}

/// 协调器接收的 owner 与查询动作。Close 幂等，等待端可以不存在或被取消。
pub(super) enum Command {
    Register(Arc<OwnerData>),
    Resolve {
        owner: OwnerId,
        provider: usize,
        waiter: ResolveWaiter,
    },
    ResolveLazy {
        owner: OwnerId,
        provider: usize,
        waiter: watch::Sender<Option<Resolution>>,
    },
    Close {
        owner: Arc<OwnerData>,
        waiter: Option<CloseWaiter>,
    },
}
