//! 宏生成构造 adapter 使用的隐藏 ABI。
//!
//! construction 只处理已经解析好的依赖如何进入一个 adapter。它不选择 provider、
//! 不缓存实例，也不管理 scope。输入遵循单向状态机：preparer 先生成不可自行构造的
//! [`PreparedInput`]，runtime 再将其写入固定长度 [`InputBuffer`](inputs::InputBuffer)。
//! 完整的 [`ConstructionInputs`] 随后要么交给 class adapter，要么由真实 lease frame
//! 包装为 [`FactoryInputs`] 后交给 factory adapter。

mod class;
mod error;
mod factory;
mod inputs;
mod preparer;
mod slot;

pub use class::ClassConstructor;
pub use error::ConstructionError;
pub use factory::{AsyncConstructor, FactoryConstructor, FactoryFuture, FactoryInputs};
pub use inputs::{ConstructionInputs, PreparedInput};
pub use preparer::{
    InputPreparer, prepare_bound_optional, prepare_bound_required, prepare_optional,
    prepare_optional_absent, prepare_required,
};
pub use slot::InputSlot;
