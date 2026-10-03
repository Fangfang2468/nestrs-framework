//! 延迟字段或 factory 参数的共享描述与单次实例上下文。
//!
//! 描述在计划装配时按依赖边创建，所有 root/scope 和该边的 Transient 消费者共享它。
//! 描述不持有 owner、实例或请求状态，因此共享不会把不同字段的初始化合并。
//! 字段只持有描述的 Arc 和 owner 的 Weak；即使 owner 已释放，错误位置和已接受
//! 请求的交付信息仍然有效，也不会形成 owner → 服务 → 字段 → owner 的强引用环。

use std::sync::{Arc, Weak};

use super::{
    InputSlot,
    projection::{ProjectionTarget, ServiceProjector},
};
use crate::{
    ResolveError,
    activation::{
        DependencyLease, Injection,
        deferred::{LazyReceiver, LazyResolver},
    },
    service::{Injectable, ServiceIdentifier, ServiceSource},
};

/// 一条已选定延迟依赖的不可变信息。归 activation 协议所有，不反向依赖 graph。
/// optional 缺席在创建字段前处理；存在此计划就必须交付一个真实实例。
#[derive(Debug)]
pub(crate) struct LazyInputPlan {
    /// 冻结计划中的 provider 节点编号。
    pub(crate) provider: usize,

    /// 消费此延迟输入的服务身份，用于追加错误路径。
    pub(crate) consumer: ServiceIdentifier,

    /// 原始声明位置，供运行期失败诊断使用。
    pub(crate) source: ServiceSource,

    /// 原字段或参数的可选名称，用于定位输入失败。
    pub(crate) label: Option<&'static str>,

    /// 共享计划中该延迟输入的原始槽位。
    pub(crate) input: InputSlot,

    /// 首次取得目标 lease 后使用的准确投影回调。
    pub(crate) project: ServiceProjector,
}

/// 当前实际字段的请求能力。克隆共享描述不复制字符串 key，也不创建新的描述分配。
#[doc(hidden)]
pub struct LazyDependency {
    /// 所有同声明消费者共享的固定延迟输入描述。
    pub(crate) plan: Arc<LazyInputPlan>,

    /// 只弱引用实际 owner 的首次请求能力，避免形成所有权环。
    pub(crate) resolver: Weak<dyn LazyResolver>,

    // 等待许可属于当前任务，必须独立于 owner 存活期；取消后已经接受的请求仍应
    // 从保存的接收端继续交付，不能为检查许可而要求已关闭的 owner 再次存活。
    /// 在等待前检查当前任务的构造上下文，不要求 owner 仍存活。
    pub(crate) check_wait_allowed: fn() -> Result<(), &'static str>,
}

impl LazyDependency {
    /// 检查当前任务是否允许等待，并将拒绝原因关联到消费者。
    pub(crate) fn check_wait_allowed(&self) -> Result<(), ResolveError> {
        (self.check_wait_allowed)().map_err(|message| self.error(message.into()))
    }

    /// 仅首次提交需要升级 owner，临时强引用不跨 await。
    pub(crate) fn request(&self) -> Result<LazyReceiver, ResolveError> {
        self.resolver
            .upgrade()
            .ok_or_else(|| self.error("服务 owner 已关闭或正在关闭，无法首次获取延迟依赖".into()))?
            .request(self.plan.provider)
            .map_err(|message| self.error(message.into()))
    }

    /// 为目标解析错误追加当前消费者的依赖路径。
    pub(crate) fn dependency_error(&self, error: ResolveError) -> ResolveError {
        ResolveError::dependency(&self.plan.consumer, self.plan.source, error)
    }

    /// 将请求或投影失败包装为带字段标签和消费者来源的错误。
    pub(crate) fn error(&self, message: String) -> ResolveError {
        let label = self.plan.label.unwrap_or("未命名输入");
        ResolveError::construction(
            &self.plan.consumer,
            self.plan.source,
            format!("延迟注入 {label}：{message}"),
        )
    }

    /// 投影直接写入真实栈上类型化接收槽，与普通构造输入共用相同边界。
    /// 返回的 Injection 自带准确 lease，既不借用临时接收端，也不依赖 owner 存活。
    pub(crate) fn prepare<T: Injectable + ?Sized>(
        &self,
        lease: DependencyLease,
    ) -> Result<Injection<T>, ResolveError> {
        ProjectionTarget::project(self.plan.input, lease, self.plan.project)
            .map_err(|error| self.error(error.to_string()))
    }
}
