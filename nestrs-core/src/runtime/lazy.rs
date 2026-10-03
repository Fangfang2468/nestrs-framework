//! 延迟字段的 owner 请求入口与等待规则。
//!
//! 这里复用 owner 原有的 Arc 分配提供弱 resolver；字段的类型化缓存与 watch 接收端
//! 由 activation 的控制块保存。runtime 只负责检查 owner 是否接受新请求、提交命令，
//! 不为每个字段再建立一份状态机，也不持有字段的缓存。

use std::sync::{Arc, Weak, atomic::Ordering};

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
    pub(super) static IN_ACTIVATION: ();
}

/// 为固定依赖交付弱 owner 引用和真实投影，不创建目标实例或新的引用计数分配。
///
/// worker 复用实际 owner 的弱请求能力；不会分配 Arc<dyn LazyResolver>。
/// 保留独立的等待检查函数，使已经接受的请求在 owner 消失后仍可接续其 watch 结果。
pub(super) fn dependency(
    resolver: &Weak<dyn LazyResolver>,
    plan: Arc<LazyInputPlan>,
) -> LazyDependency {
    LazyDependency {
        plan,
        resolver: resolver.clone(),
        check_wait_allowed,
    }
}

/// 这里只检查当前调用者，不读取 owner，也不触碰延迟字段的接收端或失败缓存。
///
/// 每个尚未取得类型化结果的调用都必须检查，包括加入另一调用已开始的初始化。
/// 缓存命中不等待新任务，由 activation 在调用本函数之前直接返回。
pub(super) fn check_wait_allowed() -> Result<(), &'static str> {
    if IN_ACTIVATION.try_with(|()| ()).is_ok() {
        return Err(
            "服务构造期间不能首次获取尚未完成的延迟注入；请声明普通注入依赖，或在服务发布后调用 LazyInjection::get",
        );
    }
    Ok(())
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
