//! 门面与中央协调器之间的异步命令通道。
//!
//! 这里负责提交、等待与句柄生命周期，不构造服务、不推进任务图。
//! 命令队列与等待端分离，使取消等待、显式关闭取消及普通 Drop 都遵守同一套关闭协议。

use super::{
    CloseWaiter, OwnerId, QueryId, Resolution, ResolveWaiter,
    coordinator::Coordinator,
    owner::{Owner, OwnerData, ROOT},
};
use crate::{
    InitializationMode,
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
    next_query: AtomicU64,
}

impl Runtime {
    pub(crate) fn start(graph: Arc<ValidatedGraph>, max: usize) -> (Arc<Self>, Arc<Owner>) {
        assert!(max > 0, "activation concurrency must be nonzero");
        let (commands, receiver) = mpsc::unbounded_channel();
        let root = OwnerData::new(ROOT, commands.downgrade());
        let owner = Arc::new(Owner {
            data: root.clone(),
            commands: commands.clone(),
        });
        let runtime = Arc::new(Self {
            graph: graph.clone(),
            commands,
            next_owner: AtomicU64::new(1),
            next_query: AtomicU64::new(0),
        });
        tokio::spawn(Coordinator::new(graph, root, receiver, max).run());
        (runtime, owner)
    }

    pub(crate) fn create_scope(&self) -> Arc<Owner> {
        let data = OwnerData::new(
            self.next_owner.fetch_add(1, Ordering::Relaxed),
            self.commands.downgrade(),
        );
        if self.commands.send(Command::Register(data.clone())).is_err() {
            data.complete_close(Err(DisposeError::coordinator_stopped()));
        }
        Arc::new(Owner {
            data,
            commands: self.commands.clone(),
        })
    }

    pub(crate) async fn resolve(&self, owner: &Owner, provider: usize) -> Resolution {
        self.request_resolution(owner, provider)?.wait().await
    }

    fn request_resolution(
        &self,
        owner: &Owner,
        provider: usize,
    ) -> Result<ResolutionRequest<'_>, ResolveError> {
        if owner.is_closed() {
            return Err(ResolveError::closed());
        }
        let query = self.next_query.fetch_add(1, Ordering::Relaxed);
        let (waiter, receiver) = oneshot::channel();
        self.commands
            .send(Command::Resolve {
                query,
                owner: owner.data.id,
                provider,
                waiter,
            })
            .map_err(|_| ResolveError::closed())?;
        Ok(ResolutionRequest {
            query,
            receiver,
            commands: &self.commands,
            completed: false,
        })
    }

    pub(crate) async fn warm_up(
        &self,
        owner: &Owner,
        lifetime: ServiceLifetime,
        default_initialization: InitializationMode,
    ) -> Result<(), ResolveError> {
        if owner.is_closed() {
            return Err(ResolveError::closed());
        }
        // 服务级策略只筛选独立预热入口。即使目标声明 lazy，它作为普通依赖被需要时，
        // 协调器仍会按原图构造；此处不删除节点、不改边，也不改变缓存和 occurrence。
        // 先提交全部选中的请求再等待，独立分支因此能同时启动；失败不放弃其他已接受任务。
        let mut receivers = Vec::new();
        for &provider in &self.graph.topological_order {
            let node = &self.graph.nodes[provider];
            let lazy = node
                .common
                .lazy
                .unwrap_or(default_initialization == InitializationMode::Lazy);
            if node.common.lifetime == lifetime && lifetime != ServiceLifetime::Transient && !lazy {
                receivers.push(self.request_resolution(owner, provider)?);
            }
        }
        let mut first_error = None;
        for receiver in receivers {
            let result = receiver.wait().await;
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
                .unwrap_or_else(|| Err(DisposeError::coordinator_stopped()));
        }
        receiver
            .await
            .unwrap_or_else(|_| Err(DisposeError::coordinator_stopped()))
    }
}

/// 普通查询的订阅所有权。取消只注销等待者，绝不取消已经接受的构造任务。
///
/// 预热也保存此凭证，因此取消预热时，正在等待及尚未轮到等待的全部请求都会退订。
/// Resolve 与退订使用同一个通道按顺序发送；若完成先到，晚到的退订是无害的空操作。
struct ResolutionRequest<'runtime> {
    query: QueryId,
    receiver: oneshot::Receiver<Resolution>,
    commands: &'runtime mpsc::UnboundedSender<Command>,
    completed: bool,
}

impl ResolutionRequest<'_> {
    async fn wait(mut self) -> Resolution {
        let result = (&mut self.receiver).await;
        self.completed = true;
        result.unwrap_or_else(|_| Err(ResolveError::closed()))
    }
}

impl Drop for ResolutionRequest<'_> {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.commands.send(Command::CancelQuery(self.query));
        }
    }
}

/// 协调器接收的 owner 与查询动作。Close 幂等，等待端可以不存在或被取消。
pub(super) enum Command {
    Register(Arc<OwnerData>),
    Resolve {
        query: QueryId,
        owner: OwnerId,
        provider: usize,
        waiter: ResolveWaiter,
    },
    CancelQuery(QueryId),
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

#[cfg(test)]
#[path = "../../tests/unit/runtime/initialization.rs"]
mod initialization_tests;

#[cfg(test)]
#[path = "../../tests/unit/runtime/requests.rs"]
mod request_tests;
