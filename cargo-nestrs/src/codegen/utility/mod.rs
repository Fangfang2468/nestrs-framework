//! 声明宏共用的语法校验与 Rust 类型检查哨兵。
//!
//! 各 element 在检查通过后透传 children，避免重复实现作用域、可见性和返回类型约束。

mod interface;
mod module_scope;
mod reject_unsafe_extern;
mod return_type;
mod visibility;

pub(crate) use interface::CheckInterfaceType;
pub(crate) use module_scope::{RequireModuleScope, impl_self_ident};
pub(crate) use reject_unsafe_extern::{RejectUnsafeAndExternFn, RejectUnsafeImpl};
pub(crate) use return_type::{
    RequireNonUnitFutureOutputType, RequireNonUnitResultOkType, RequireNonUnitReturnType,
};
pub(crate) use visibility::MustBePrivateFn;
