//! Factory 构造适配器的借用协议。
//!
//! class 直接把 `Injection<T>` 移入服务字段；factory 的普通参数改写成 `&T`，因此
//! 需要独立调用帧保留全部 lease。worker 先拥有帧，再借出输入并等待 future，最后把
//! 帧的依赖转交给成功实例。借用完全来自真实帧，不把局部输入延长成 `'static`。
//! 延迟参数则按值移交 `LazyInjection<T>`：该输入尚未请求并取得目标，不能向帧借出 `&T`；
//! 目标可能已由其他查询或初始化入口构造，但本槽位仍需通过句柄独立完成获取。
//! 句柄自带独立槽位和弱 owner 请求能力，可以跨 await 并保存到最终返回的服务中。

use std::{future::Future, pin::Pin};

use super::{ConstructionError, ConstructionInputs, InputSlot};
use crate::{
    activation::{DependencyLease, LazyInjection, erased_service::ErasedService},
    service::Injectable,
};

/// 异步工厂适配器返回的 future，其存活期不能超过借入的调用帧。
pub type FactoryFuture<'frame> =
    Pin<Box<dyn Future<Output = Result<ErasedService, ConstructionError>> + Send + 'frame>>;

/// 同步工厂适配器的单态化签名；每次调用接受任意合法的帧借用期。
pub type FactoryConstructor =
    for<'frame> fn(FactoryInputs<'frame>) -> Result<ErasedService, ConstructionError>;

/// 异步工厂适配器的单态化签名；future 沿用输入的帧借用期。
pub type AsyncConstructor = for<'frame> fn(FactoryInputs<'frame>) -> FactoryFuture<'frame>;

/// 仅供工厂适配器消费、绑定调用帧借用期的构造输入。
///
/// 只有持有全部真实依赖 lease 的 `FactoryLeaseFrame` 可以创建它。
#[doc(hidden)]
pub struct FactoryInputs<'frame> {
    /// 已从帧移交、供 adapter 按槽位消费的完整输入。
    inputs: ConstructionInputs,

    /// 把普通参数借用期绑定到仍持有全部依赖的帧。
    _lease_frame: &'frame [DependencyLease],
}

/// 由 worker 持有、但只能交付一次输入的工厂调用帧。
pub(crate) struct FactoryLeaseFrame {
    /// 尚未移交的完整输入；工厂帧只允许交付一次。
    inputs: Option<ConstructionInputs>,

    /// 从原始立即输入派生的真实保活集合，构造成功后移交实例。
    dependencies: Vec<DependencyLease>,
}

impl FactoryLeaseFrame {
    /// 从未消费的输入派生全部真实 lease，建立唯一的工厂调用帧。
    pub(crate) fn new(inputs: ConstructionInputs) -> Self {
        // 保活对象只能来自这些尚未消费的准确输入，调用者不能另传不匹配的 lease。
        let dependencies = inputs.dependency_leases();
        Self {
            inputs: Some(inputs),
            dependencies,
        }
    }

    /// 移交一次构造输入，并将普通参数的借用期绑定到当前帧。
    pub(crate) fn inputs(&mut self) -> FactoryInputs<'_> {
        FactoryInputs {
            inputs: self.inputs.take().expect("工厂帧只能交付一次构造输入"),
            _lease_frame: &self.dependencies,
        }
    }

    /// 构造结束后移交帧保存的依赖，供成功实例继续保活。
    pub(crate) fn into_dependencies(self) -> Vec<DependencyLease> {
        self.dependencies
    }
}

impl<'frame> FactoryInputs<'frame> {
    /// 取走一个只在本次 factory 调用期间有效的必选服务借用。
    pub fn take<T>(&mut self, slot: InputSlot) -> Result<&'frame T, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.inputs.take::<T>(slot)?;

        // SAFETY: frame 从这些输入派生全部真实 lease；take 的投影还验证返回令牌与
        // 原输入属于同一实例。frame 的借用保持到 'frame，释放临时 token 不影响地址。
        Ok(unsafe { token.into_ptr().as_ref() })
    }

    /// 取走一个只在本次 factory 调用期间有效的可选服务借用。
    pub fn take_optional<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<&'frame T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.inputs.take_optional::<T>(slot)?;

        Ok(token.map(|token| {
            // SAFETY: 与 take 相同，投影核对同一实例且帧为 'frame 保活；缺席没有地址。
            unsafe { token.into_ptr().as_ref() }
        }))
    }

    /// 移交一个必选延迟参数；消费输入不会立即解析或构造目标。
    ///
    /// 句柄拥有自己的槽位，并不借用 factory frame，因此可以安全地移入返回服务。
    /// 目标首次交付后由句柄持有真实 lease；此处不伪造实例引用，也不延长帧的借用期。
    pub fn take_lazy<T>(&mut self, slot: InputSlot) -> Result<LazyInjection<T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.inputs.take_lazy(slot)
    }

    /// 移交一个可选延迟参数；None 表示冻结图没有候选，不表示目标构造失败。
    /// 与必选参数一样，所有权随句柄移交；剩余输入和普通依赖仍由原调用帧管理。
    pub fn take_optional_lazy<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<LazyInjection<T>>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.inputs.take_optional_lazy(slot)
    }

    /// 拒绝 factory adapter 未消费的 descriptor 槽位。
    pub fn ensure_all_consumed(&self) -> Result<(), ConstructionError> {
        self.inputs.ensure_all_consumed()
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/factory.rs"]
mod tests;
