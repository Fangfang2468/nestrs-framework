mod async_fn;
mod constructor;
mod interface;
mod module_scope;
mod reject_unsafe_extern;
mod return_type;
mod visibility;

pub(crate) use async_fn::ShouldBeAsyncFn;
pub(crate) use constructor::CheckConstructor;
pub(crate) use interface::CheckInterfaceType;
pub(crate) use module_scope::{impl_self_ident, RequireModuleScope};
pub(crate) use reject_unsafe_extern::{RejectUnsafeAndExternFn, RejectUnsafeImpl};
pub(crate) use return_type::{
    RequireNonUnitFutureOutputType, RequireNonUnitResultOkType, RequireNonUnitReturnType,
};
pub(crate) use visibility::MustBePrivateFn;
