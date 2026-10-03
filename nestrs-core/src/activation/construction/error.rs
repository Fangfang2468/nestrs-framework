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
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ConstructionError {
    /// 请求的输入编号超出完整输入集合。
    #[error("构造输入槽位 {slot:?} 超出范围（槽位总数：{slot_count}）")]
    SlotOutOfBounds {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,

        /// 当前完整输入集合的槽位数量。
        slot_count: usize,
    },

    /// 计划与交付上下文记录的输入编号不同。
    #[error("构造输入槽位 {slot:?} 的描述指向了槽位 {actual:?}")]
    InputSlotMismatch {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,

        /// 执行描述记录的实际槽位编号。
        actual: InputSlot,
    },

    /// 缺席或延迟槽位错误携带已构造实例。
    #[error("构造输入槽位 {slot:?} 不应包含已构造的依赖实例")]
    UnexpectedDependencyPresent {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 同一投影接收槽被重复写入。
    #[error("构造输入槽位 {slot:?} 已经准备")]
    SlotAlreadyPrepared {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 必需的输入或投影结果尚未交付。
    #[error("构造输入槽位 {slot:?} 尚未准备")]
    UnfilledSlot {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// adapter 重复读取已经移交的输入。
    #[error("构造输入槽位 {slot:?} 已被消费")]
    SlotAlreadyConsumed {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 读取操作要求普通必选输入，但槽位形态不同。
    #[error("构造输入槽位 {slot:?} 需要必选依赖")]
    RequiredInputExpected {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 读取操作要求普通可选输入，但槽位形态不同。
    #[error("构造输入槽位 {slot:?} 需要可选依赖")]
    OptionalInputExpected {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 读取操作要求延迟必选输入，但槽位形态不同。
    #[error("构造输入槽位 {slot:?} 需要必选延迟依赖")]
    LazyRequiredInputExpected {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 读取操作要求延迟可选输入，但槽位形态不同。
    #[error("构造输入槽位 {slot:?} 需要可选延迟依赖")]
    LazyOptionalInputExpected {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 必选输入的冻结选择或交付结果缺少目标。
    #[error("构造输入槽位 {slot:?} 的必选依赖不存在")]
    RequiredDependencyAbsent {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 准确 Rust 类型与请求或接收槽不一致。
    #[error("构造输入槽位 {slot:?} 的类型不匹配：期望 {expected}，实际为 {actual}")]
    InputTypeMismatch {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,

        /// 读取或投影要求的准确类型名。
        expected: &'static str,

        /// 输入或投影实际携带的类型名。
        actual: &'static str,
    },

    /// 投影交付了另一个实例，而非原输入 lease 对应实例。
    #[error("构造输入槽位 {slot:?} 的投影返回了不同实例的注入令牌")]
    ProjectionOwnerMismatch {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 执行用户构造前仍有输入没有被读取。
    #[error("构造输入槽位 {slot:?} 仍未被消费")]
    UnconsumedSlot {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,
    },

    /// 接口输入没有配套的真实投影能力。
    #[error("构造输入槽位 {slot:?} 的 trait 类型 {trait_type} 缺少 concrete-to-trait 投影")]
    UnprojectedTraitInput {
        /// 请求读取或交付的输入槽位编号。
        slot: InputSlot,

        /// 尚未取得真实投影能力的接口类型名。
        trait_type: &'static str,
    },

    /// 用户工厂返回错误，保留其 Debug 详情。
    #[error("factory provider {provider}（{provider_source:?}）执行失败：{detail}")]
    FactoryFailed {
        /// 返回错误的业务工厂函数名。
        provider: &'static str,

        /// 发生构造失败的服务声明位置。
        provider_source: ServiceSource,

        /// 保留用户错误的 Debug 文本，供解析错误关联依赖路径。
        detail: String,
    },

    /// 用户显式构造函数返回错误，保留其 Debug 详情。
    #[error("constructor provider {provider}（{provider_source:?}）执行失败：{detail}")]
    ConstructorFailed {
        /// 发生构造失败的服务类型名。
        provider: &'static str,

        /// 发生构造失败的服务声明位置。
        provider_source: ServiceSource,

        /// 保留用户错误的 Debug 文本，供解析错误关联依赖路径。
        detail: String,
    },
}
