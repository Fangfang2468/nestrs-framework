//! 服务实例的构造协议与内存安全边界。
//!
//! 本模块接收已经选定的依赖，把普通输入的实例或延迟输入的句柄交给构造适配器，并
//! 维持结果的地址和依赖存活期。它不选择 provider、不展开依赖图，也不管理 scope；
//! 这些职责由图编译器和运行时承担。延迟字段只缓存自身的类型化 token，运行时合并
//! 初始化的职责通过内部 trait 注入，不能反向依赖注册描述或 runtime 模块。
//!
//! 代码按以下职责组织：
//! - [`construction`]：准备输入、检查槽位、执行 class / factory 各自的交付协议。
//! - [`instance`]、[`injection`] 与 [`lazy`]：用强 lease 保活稳定实例，交付普通或延迟令牌。
//! - [`release`]：在最后一个 lease 消失后迭代销毁实例，避免深依赖链递归析构。
//!
//! 所有指针恢复都依赖三个不变量：实例装入 Box 后不再移动；可解引用的令牌或工厂帧
//! 持有准确实例的强 lease；向业务返回 owner 借用期的引用前，运行时已把实例收纳进
//! owner 的记录中。class 把令牌移入字段，factory 则借用真实调用帧，二者不能用伪造的
//! `'static` 借用合并。内部类型的公开重导出仅供编译器生成适配器使用。

pub(crate) mod construction;
pub(crate) mod erased_service;
pub(crate) mod injection;
mod instance;
pub(crate) mod lazy;
mod release;

pub(crate) use construction::ActivationPreparation;
pub use construction::{
    AsyncConstructor, ClassConstructor, ConstructionError, ConstructionInputs, FactoryConstructor,
    FactoryFuture, FactoryInputs, InputPreparer, InputSlot, LazyInputPreparer, PreparedInput,
    prepare_bound_optional, prepare_bound_required, prepare_lazy_optional, prepare_lazy_required,
    prepare_optional, prepare_optional_absent, prepare_required,
};
pub use erased_service::{ErasedService, ErasedServiceRef};
pub use injection::Injection;
pub(crate) use instance::DependencyLease;
pub use lazy::LazyInjection;
pub(crate) use release::ReleaseDomain;
