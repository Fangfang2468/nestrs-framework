//! 编译器生成构造适配器使用的内部协议。
//!
//! construction 只处理已经解析好的依赖如何进入一个 adapter。它不选择 provider、
//! 不缓存实例，也不管理 scope。runtime 一次交付完整的 [`ConstructionInputs`]，
//! class adapter 直接取得拥有 lease 的令牌；factory adapter 经真实 lease frame
//! 包装的 [`FactoryInputs`] 取得调用期借用。
//!
//! [`inputs`] 验证完整来源、准确形态和单次消费；[`projection`] 直接向栈上类型化
//! 接收槽交付令牌并检查真实实例身份。构造、根查询与 lazy 目标共用这一个投影边界，
//! 不为各参数创建装箱的中间值。生成 adapter 先读取并验证全部参数，再执行用户代码；
//! 输入消费不能修改注册、选择依赖或提前请求 lazy 目标。

mod class;
mod error;
mod factory;
mod inputs;
mod lazy_dependency;
mod projection;
mod slot;

pub use class::ClassConstructor;
pub use error::ConstructionError;
pub(crate) use factory::FactoryLeaseFrame;
pub use factory::{AsyncConstructor, FactoryConstructor, FactoryFuture, FactoryInputs};
pub(crate) use inputs::ConstructionInput;
pub use inputs::{ConstructionInputs, InputKind};
pub use lazy_dependency::LazyDependency;
pub(crate) use lazy_dependency::LazyInputPlan;
pub use projection::{ProjectionTarget, ServiceProjector, project_bound, project_required};
pub use slot::InputSlot;
