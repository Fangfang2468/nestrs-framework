//! 延迟字段的 owner 请求入口与等待规则。
//!
//! 这里复用 owner 原有的 Arc 分配提供弱 resolver；字段的类型化缓存与 watch 接收端
//! 由 activation 的控制块保存。runtime 只负责检查 owner 是否接受新请求、提交命令，
//! 不为每个字段再建立一份状态机，也不持有字段的缓存。

use std::{
    future::Future,
    sync::{Arc, Weak, atomic::Ordering},
};

use tokio::sync::watch;

use super::{handle::Command, owner::OwnerData};
use crate::activation::{
    LazyDependency, LazyInputPlan,
    deferred::{LazyReceiver, LazyResolver},
};

tokio::task_local! {
    /// 构造 worker 持有全局激活名额；此时等待新的延迟目标可能耗尽名额而死锁。
    ///
    /// 标记属于 runtime 的调度约束，activation 仅通过等待许可函数访问它。
    /// 它覆盖完整构造 future，随任务而非操作系统线程移动，不传播到用户自行 spawn 的任务。
    static IN_ACTIVATION: ();
}

/// 一次激活的 owner 弱能力与延迟等待规则。
///
/// 复用实际 owner 的 Arc 分配，不增加 resolver 分配，也不保存字段的初始化状态。
/// 等待许可只依赖当前任务，使已接受的请求在 owner 消失后仍可接续 watch 结果。
pub(super) struct ActivationContext {
    resolver: Weak<dyn LazyResolver>,
}

impl ActivationContext {
    pub(super) fn new(resolver: Weak<dyn LazyResolver>) -> Self {
        Self { resolver }
    }

    pub(super) async fn run<F: Future>(future: F) -> F::Output {
        IN_ACTIVATION.scope((), future).await
    }

    /// 为当前槽位组合固定计划与实际 owner；不提交请求或创建目标实例。
    pub(super) fn dependency(&self, plan: Arc<LazyInputPlan>) -> LazyDependency {
        LazyDependency {
            plan,
            resolver: self.resolver.clone(),
            check_wait_allowed: Self::check_wait_allowed,
        }
    }

    /// 不读取 owner 或字段缓存；每个尚未取得类型化结果的调用都要检查。
    /// 缓存命中不等待新任务，由 activation 在调用本函数之前直接返回。
    pub(super) fn check_wait_allowed() -> Result<(), &'static str> {
        if IN_ACTIVATION.try_with(|()| ()).is_ok() {
            return Err(
                "服务构造期间不能首次获取尚未完成的延迟注入；请声明普通注入依赖，或在服务发布后调用 LazyInjection::get",
            );
        }
        Ok(())
    }
}

impl LazyResolver for OwnerData {
    /// 同步提交一次请求；单字段的去重与取消接续由 activation 控制块保证。
    ///
    /// 调用期间暂时升级命令通道，返回后只留下接收端，不持有 owner 或 runtime 的
    /// 强引用。关闭与请求之间若有竞争，协调器仍按队列中的 owner 状态作最终裁决。
    fn request(&self, provider: usize) -> Result<LazyReceiver, &'static str> {
        const CLOSED: &str = "服务 owner 已关闭或正在关闭，无法首次获取延迟依赖";
        if self.status.load(Ordering::Acquire) != super::owner::OPEN {
            return Err(CLOSED);
        }
        let commands = self.commands.upgrade().ok_or(CLOSED)?;
        let (waiter, receiver) = watch::channel(None);
        commands
            .send(Command::ResolveLazy {
                owner: self.id,
                provider,
                waiter,
            })
            .map_err(|_| CLOSED)?;
        Ok(receiver)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/lazy.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/unit/runtime/lazy_delivery.rs"]
mod delivery_tests;
