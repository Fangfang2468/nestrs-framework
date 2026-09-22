//! 服务构造期的内部 ABI。
//!
//! activation 只依赖 [`crate::service`]：它接收已经解析好的服务引用，把它们交给宏
//! 生成的 adapter，并定义构造结果的擦除表示。provider 注册 metadata 位于依赖
//! activation 的 `registration` 模块，本模块不能反向引用它。

pub(crate) mod construction;
pub(crate) mod erased_service;
pub(crate) mod injection;

pub use construction::{
    AsyncConstructor, ClassConstructor, ConstructionError, ConstructionInputs, FactoryConstructor,
    FactoryFuture, FactoryInputs, InputPreparer, InputSlot, PreparedInput, prepare_bound_optional,
    prepare_bound_required, prepare_optional, prepare_optional_absent, prepare_required,
};
pub use erased_service::{ErasedService, ErasedServiceRef};
pub use injection::Injection;
