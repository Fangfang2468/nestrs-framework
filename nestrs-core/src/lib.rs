mod inject_wrapper;
pub mod lifetime;
mod metadata;
pub mod registration;


#[doc(hidden)]
pub mod __private {
    /// 仅供宏生成的分布式注册静态项使用的 `linkme` crate 重导出。
    ///
    /// 下游应用无需也不应为了 DI 注册而直接依赖此名称。
    pub use linkme;

    /// 仅供宏展开引用的依赖令牌类型。
    pub use crate::inject_wrapper::Inject;
    pub use crate::metadata::{
        impl_bind::{InterfaceBinding, REFLECT_METADATA_BIND},
        injectable::{StructComponent, FieldInjection, REFLECT_METADATA_INJECTABLE},
    };
}
