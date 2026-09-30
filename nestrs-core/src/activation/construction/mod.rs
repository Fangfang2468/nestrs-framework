//! 编译器生成构造适配器使用的内部协议。
//!
//! construction 只处理已经解析好的依赖如何进入一个 adapter。它不选择 provider、
//! 不缓存实例，也不管理 scope。输入遵循单向状态机：preparer 先生成不可自行构造的
//! [`PreparedInput`]，runtime 再将其写入固定长度 [`InputBuffer`](preparation::InputBuffer)。
//! 完整的 [`ConstructionInputs`] 随后要么交给 class adapter，要么由真实 lease frame
//! 包装为 [`FactoryInputs`] 后交给 factory adapter。
//!
//! 阅读顺序：[`preparer`] 定义类型检查和 trait 投影，[`preparation`] 定义事务式填充，
//! [`inputs`] 定义单次消费，最后由 [`class`] 或 [`factory`] 交付相应所有权。每个阶段
//! 只暴露下一阶段需要的能力，避免适配器边构造边修改注册或依赖选择。

mod class;
mod error;
mod factory;
mod inputs;
mod preparation;
mod preparer;
mod slot;

pub use class::ClassConstructor;
pub use error::ConstructionError;
pub(crate) use factory::FactoryLeaseFrame;
pub use factory::{AsyncConstructor, FactoryConstructor, FactoryFuture, FactoryInputs};
pub use inputs::{ConstructionInputs, PreparedInput};
pub(crate) use preparation::ActivationPreparation;
pub use preparer::{
    InputPreparer, prepare_bound_optional, prepare_bound_required, prepare_optional,
    prepare_optional_absent, prepare_required,
};
pub use slot::InputSlot;
