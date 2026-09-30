//! 构造期输入状态机的内部错误。

use thiserror::Error;

use crate::service::ServiceSource;

use super::slot::InputSlot;

/// 由输入准备、adapter 消费或 factory 调用产生的受控错误。
///
/// 此错误属于内部构造协议，不直接作为公开错误暴露。运行时在激活边界补上 provider、
/// 源码位置和依赖路径，转换为公开的 [`crate::ResolveError`]；Eager 构建再按构建契约
/// 汇总为 [`crate::BuildError`]。
#[doc(hidden)]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConstructionError {
    #[error("构造输入槽位 {slot:?} 超出范围（槽位总数：{slot_count}）")]
    SlotOutOfBounds { slot: InputSlot, slot_count: usize },

    #[error("构造输入槽位 {slot:?} 已经准备")]
    SlotAlreadyPrepared { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 尚未准备")]
    UnfilledSlot { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 已被消费")]
    SlotAlreadyConsumed { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 需要必选依赖")]
    RequiredInputExpected { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 需要可选依赖")]
    OptionalInputExpected { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 的必选依赖不存在")]
    RequiredDependencyAbsent { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 的类型不匹配：期望 {expected}，实际为 {actual}")]
    InputTypeMismatch {
        slot: InputSlot,
        expected: &'static str,
        actual: &'static str,
    },

    #[error("构造输入槽位 {slot:?} 仍未被消费")]
    UnconsumedSlot { slot: InputSlot },

    #[error("构造输入槽位 {slot:?} 的 trait 类型 {trait_type} 缺少 concrete-to-trait 投影")]
    UnprojectedTraitInput {
        slot: InputSlot,
        trait_type: &'static str,
    },

    #[error("factory provider {provider}（{provider_source:?}）执行失败：{detail}")]
    FactoryFailed {
        provider: &'static str,
        provider_source: ServiceSource,
        detail: String,
    },
}
