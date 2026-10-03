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
    /// 当前入口共享的不可变执行计划。
    graph: Arc<ValidatedGraph>,

    /// 向当前 root 协调器提交命令的通道。
    commands: mpsc::UnboundedSender<Command>,

    /// 为新 scope 分配互不冲突的 owner 编号。
    next_owner: AtomicU64,

    /// 为普通查询订阅分配唯一编号，取消时据此定位。
    next_query: AtomicU64,
}

impl Runtime {
    /// 建立 root、命令通道和中央协调任务，设置共享构造上限。
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

    /// 创建独立 Scoped owner 并提交注册命令，不构造服务。
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

    /// 提交一次普通解析请求，并等待其缓存或构造结果。
    pub(crate) async fn resolve(&self, owner: &Owner, provider: usize) -> Resolution {
        self.request_resolution(owner, provider)?.wait().await
    }

    /// 在第一次挂起前分配查询编号并提交命令，返回负责退订的等待凭证。
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

    /// 按生命周期和服务级覆盖选择预热入口，先提交全部请求再等待结果。
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

    /// 提交幂等关闭请求并等待最终结果；已完成关闭时直接复用结果。
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
    /// 当前普通查询的订阅编号。
    query: QueryId,

    /// 等待当前解析结果的一次性接收端。
    receiver: oneshot::Receiver<Resolution>,

    /// 向当前 root 协调器提交命令的通道。
    commands: &'runtime mpsc::UnboundedSender<Command>,

    /// 是否已收到最终结果，决定 Drop 时是否需要退订。
    completed: bool,
}

impl ResolutionRequest<'_> {
    /// 等待一次查询交付，并标记完成以避免多余退订。
    async fn wait(mut self) -> Resolution {
        let result = (&mut self.receiver).await;
        self.completed = true;
        result.unwrap_or_else(|_| Err(ResolveError::closed()))
    }
}

impl Drop for ResolutionRequest<'_> {
    /// 取消尚未完成的普通等待订阅，已接受的构造任务继续执行。
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.commands.send(Command::CancelQuery(self.query));
        }
    }
}

/// 协调器接收的 owner 与查询动作。Close 幂等，等待端可以不存在或被取消。
pub(super) enum Command {
    /// 登记新 scope 的共享 owner 数据，不触发服务构造。
    Register(Arc<OwnerData>),

    /// 提交普通查询及其可取消的一次性订阅。
    Resolve {
        /// 当前普通查询的订阅编号。
        query: QueryId,

        /// 本次操作所属的实际 root 或 scope。
        owner: OwnerId,

        /// 冻结计划中的 provider 节点编号。
        provider: usize,

        /// 用于交付该命令结果的发送端；接收端取消不撤销已接受工作。
        waiter: ResolveWaiter,
    },

    /// 注销指定普通查询的等待者，不取消已接受构造。
    CancelQuery(QueryId),

    /// 提交字段独立的延迟消费，结果由可接续订阅交付。
    ResolveLazy {
        /// 本次操作所属的实际 root 或 scope。
        owner: OwnerId,

        /// 冻结计划中的 provider 节点编号。
        provider: usize,

        /// 用于交付该命令结果的发送端；接收端取消不撤销已接受工作。
        waiter: watch::Sender<Option<Resolution>>,
    },

    /// 幂等请求 owner 排空并清理，可附带完成等待者。
    Close {
        /// 本次操作所属的实际 root 或 scope。
        owner: Arc<OwnerData>,

        /// 用于交付该命令结果的发送端；接收端取消不撤销已接受工作。
        waiter: Option<CloseWaiter>,
    },
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/initialization.rs"]
mod initialization_tests;

#[cfg(test)]
#[path = "../../tests/unit/runtime/requests.rs"]
mod request_tests;
