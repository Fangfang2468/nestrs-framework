//! 每个 root 一个、通过显式任务图推进的 Tokio 运行时。
//!
//! 阅读路径：`handle` 提交命令 → `coordinator` 展开任务并调度 → `worker` 执行一个节点。
//! `owner` 定义缓存、发布 journal 与关闭阶段，`task` 定义尚未完成的构造 occurrence。
//!
//! 三个关键不变量：
//! - 只有协调器修改任务图和 owner 状态；worker 永远不递归调用 resolver。
//! - 成功实例先进入实际 owner 的 journal，再通知等待者或推进消费者。
//! - 关闭先排空已接受任务，再逐 owner 按消费者优先的依赖顺序完成 cleanup；内存保活独立于 Tokio。

mod cleanup;
mod compact;
mod coordinator;
mod handle;
mod lazy;
mod owner;
mod task;
mod worker;

use crate::{
    activation::DependencyLease,
    error::{DisposeError, ResolveError},
};
use tokio::sync::oneshot;

pub(crate) use handle::Runtime;
pub(crate) use owner::Owner;

type OwnerId = u64;
type TaskId = u64;
type QueryId = u64;
type Resolution = Result<DependencyLease, ResolveError>;
type ResolveWaiter = oneshot::Sender<Resolution>;
type CloseWaiter = oneshot::Sender<Result<(), DisposeError>>;
