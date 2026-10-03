//! 已准备输入的交付与消费，不包含准备阶段的写入权限。
//!
//! [`PreparedInput`] 保存一个已经校验类型、带有真实 lease 的注入令牌；
//! [`ConstructionInputs`] 则是一组只能各消费一次的固定槽位。必选与可选只在载荷形态
//! 上不同，统一经过“检查形态和类型，再移出载荷”的流程。任何检查失败都保留原槽位，
//! 因而诊断错误不会顺带销毁一个仍可正确读取的输入。

use std::{any::Any, mem};

use crate::{
    activation::{DependencyLease, Injection, LazyInjection},
    service::Injectable,
};

use super::{error::ConstructionError, slot::InputSlot};

/// 已经完成类型化准备、但尚未写入固定槽位的一个输入。
///
/// `InputPreparer` 只会产生此值；实际写入缓冲区与收纳依赖 lease 的动作
/// 由 `ActivationPreparation` 集中完成，因此 preparer 无法留下半写入状态。
#[doc(hidden)]
pub struct PreparedInput {
    kind: InputKind,
    value: Box<dyn Any + Send + Sync>,
    service_type_name: &'static str,
    // 令牌自身持有一份 lease；这一份供准备阶段交给实例或工厂帧独立保活。
    // 不能根据 preparer 的入参推断 owner，因为返回令牌可能来自它保留的另一实例。
    lease: Option<DependencyLease>,
}

/// 输入载荷的交付形态，与“槽位是否已消费”是两个不同维度。
///
/// 只有本模块的私有构造函数能组合形态、真实载荷类型与 lease，生成适配器不能伪造。
#[derive(Clone, Copy, PartialEq, Eq)]
enum InputKind {
    Required,
    Optional,
    LazyRequired,
    LazyOptional,
}

impl PreparedInput {
    /// 包装已经带有真实 owner lease 的必选注入 token。
    pub(super) fn required<T>(token: Injection<T>) -> Self
    where
        T: Injectable + ?Sized,
    {
        Self {
            lease: Some(token.lease()),
            kind: InputKind::Required,
            value: Box::new(token),
            service_type_name: std::any::type_name::<T>(),
        }
    }

    /// 包装可选 token；`None` 是已准备的缺席值，而非未填充槽位。
    pub(super) fn optional<T>(token: Option<Injection<T>>) -> Self
    where
        T: Injectable + ?Sized,
    {
        Self {
            lease: token.as_ref().map(Injection::lease),
            kind: InputKind::Optional,
            value: Box::new(token),
            service_type_name: std::any::type_name::<T>(),
        }
    }

    /// 延迟句柄尚无实例 lease；成功获取时由句柄自己收纳真实 token。
    pub(super) fn lazy_required<T>(token: LazyInjection<T>) -> Self
    where
        T: Injectable + ?Sized,
    {
        Self {
            lease: None,
            kind: InputKind::LazyRequired,
            value: Box::new(token),
            service_type_name: std::any::type_name::<T>(),
        }
    }

    pub(super) fn lazy_optional<T>(token: Option<LazyInjection<T>>) -> Self
    where
        T: Injectable + ?Sized,
    {
        Self {
            lease: None,
            kind: InputKind::LazyOptional,
            value: Box::new(token),
            service_type_name: std::any::type_name::<T>(),
        }
    }

    pub(super) fn dependency(&self) -> Option<DependencyLease> {
        self.lease.clone()
    }

    /// 直接消费一个已准备的必选载荷，供构造协议的隔离校验使用。
    /// 根查询与延迟目标交付使用 project_token，不再经过此装箱输入路径。
    pub(crate) fn into_required<T>(self, slot: InputSlot) -> Result<Injection<T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.validate::<Injection<T>>(slot, InputKind::Required, std::any::type_name::<T>())?;
        Ok(self.into_value())
    }

    /// 直接消费一个已准备的可选载荷；缺席和存在都必须匹配准确的 Option 类型。
    pub(crate) fn into_optional<T>(
        self,
        slot: InputSlot,
    ) -> Result<Option<Injection<T>>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.validate::<Option<Injection<T>>>(
            slot,
            InputKind::Optional,
            std::any::type_name::<T>(),
        )?;
        Ok(self.into_value())
    }

    /// 只读验证；调用者确认成功后，才可以改变槽位的消费状态。
    fn validate<Value: Any>(
        &self,
        slot: InputSlot,
        kind: InputKind,
        expected: &'static str,
    ) -> Result<(), ConstructionError> {
        if self.kind != kind {
            return Err(match kind {
                InputKind::Required => ConstructionError::RequiredInputExpected { slot },
                InputKind::Optional => ConstructionError::OptionalInputExpected { slot },
                InputKind::LazyRequired => ConstructionError::LazyRequiredInputExpected { slot },
                InputKind::LazyOptional => ConstructionError::LazyOptionalInputExpected { slot },
            });
        }
        if !self.value.is::<Value>() {
            return Err(ConstructionError::InputTypeMismatch {
                slot,
                expected,
                actual: self.service_type_name,
            });
        }
        Ok(())
    }

    /// 仅接收已经通过 `validate::<Value>` 的载荷；令牌自己的 lease 随值一起移出。
    fn into_value<Value: Any>(self) -> Value {
        *self
            .value
            .downcast::<Value>()
            .expect("输入载荷必须在移出前完成准确类型检查")
    }
}

/// 已完成绑定的构造输入。
///
/// class 适配器直接消费它；factory 适配器仅通过持有真实 lease 的
/// [`super::FactoryInputs`] 消费它。该类型不提供写入 API；adapter 只能按 slot 取得
/// 匹配的必选或可选令牌，并在构造完成前通过
/// [`Self::ensure_all_consumed`] 验证描述中的依赖与适配器实际读取一致。
#[doc(hidden)]
pub struct ConstructionInputs {
    slots: Vec<ConsumptionSlot>,
}

enum ConsumptionSlot {
    // 可选输入的 None 也属于 Available；它和已经取走的 Consumed 不同。
    Available(PreparedInput),
    Consumed,
}

impl ConstructionInputs {
    /// 创建没有依赖的合法构造输入，供零依赖 adapter 使用。
    pub fn empty() -> Self {
        Self { slots: Vec::new() }
    }

    /// 仅供准备阶段使用；每个元素都对应已填满且校验过边界的固定槽位。
    pub(super) fn from_prepared(inputs: Vec<PreparedInput>) -> Self {
        Self {
            slots: inputs.into_iter().map(ConsumptionSlot::Available).collect(),
        }
    }

    /// 取走一个必选字段注入 token。
    pub fn take<T>(&mut self, slot: InputSlot) -> Result<Injection<T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.take_value(slot, InputKind::Required, std::any::type_name::<T>())
    }

    /// 取走一个可选字段注入 token。
    pub fn take_optional<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<Injection<T>>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.take_value(slot, InputKind::Optional, std::any::type_name::<T>())
    }

    /// 移交必选延迟字段；读取句柄本身不会启动目标构造。
    pub fn take_lazy<T>(&mut self, slot: InputSlot) -> Result<LazyInjection<T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.take_value(slot, InputKind::LazyRequired, std::any::type_name::<T>())
    }

    /// 移交可选延迟字段；缺席只由冻结图决定，目标初始化失败不会伪装成 `None`。
    pub fn take_optional_lazy<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<LazyInjection<T>>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.take_value(slot, InputKind::LazyOptional, std::any::type_name::<T>())
    }

    /// 拒绝仍遗留在 adapter 输入中的槽位。
    pub fn ensure_all_consumed(&self) -> Result<(), ConstructionError> {
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| matches!(slot, ConsumptionSlot::Available(_)))
        else {
            return Ok(());
        };

        Err(ConstructionError::UnconsumedSlot {
            slot: InputSlot::new(index),
        })
    }

    /// 必选和可选共享此流程，保证所有错误检查都发生在消费槽位之前。
    fn take_value<Value: Any>(
        &mut self,
        slot: InputSlot,
        kind: InputKind,
        expected: &'static str,
    ) -> Result<Value, ConstructionError> {
        self.available(slot)?
            .validate::<Value>(slot, kind, expected)?;
        Ok(self.take_available(slot).into_value())
    }

    fn available(&self, slot: InputSlot) -> Result<&PreparedInput, ConstructionError> {
        let slot_count = self.slots.len();
        let Some(value) = self.slots.get(slot.index()) else {
            return Err(ConstructionError::SlotOutOfBounds { slot, slot_count });
        };

        match value {
            ConsumptionSlot::Available(input) => Ok(input),
            ConsumptionSlot::Consumed => Err(ConstructionError::SlotAlreadyConsumed { slot }),
        }
    }

    fn take_available(&mut self, slot: InputSlot) -> PreparedInput {
        let value = mem::replace(
            self.slots
                .get_mut(slot.index())
                .expect("槽位必须在消费前完成范围检查"),
            ConsumptionSlot::Consumed,
        );

        let ConsumptionSlot::Available(input) = value else {
            unreachable!("槽位必须在消费前确认尚未被消费");
        };

        input
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/inputs.rs"]
mod tests;
