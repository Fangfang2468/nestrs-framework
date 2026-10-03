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

/// 同一 root 运行时中的 owner 编号，root 使用保留值零。
type OwnerId = u64;

/// 一次实际构造 occurrence 的编号，与静态 provider 编号分开。
type TaskId = u64;

/// 普通查询订阅的唯一编号，供取消等待时精确退订。
type QueryId = u64;

/// 一次解析交付的真实实例 lease 或带依赖来源的错误。
type Resolution = Result<DependencyLease, ResolveError>;

/// 普通查询的一次性结果发送端。
type ResolveWaiter = oneshot::Sender<Resolution>;

/// 等待 owner 完整关闭结果的一次性发送端。
type CloseWaiter = oneshot::Sender<Result<(), DisposeError>>;
