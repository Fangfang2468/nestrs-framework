pub mod arena;
mod construction;
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

    /// 宏生成字段输入 adapter 所使用的稳定 Arena 引用类型。
    pub use crate::arena::ArenaServiceRef;
    /// 仅供宏生成构造 adapter 与 core activation runtime 共用的隐藏 ABI。
    pub use crate::construction::{
        ActivationError, ConstructionContext, Constructor, ErasedService, InputPosition,
        PrepareInput, prepare_bound_optional, prepare_bound_required, prepare_optional,
        prepare_optional_absent, prepare_required,
    };
    /// 仅供宏展开引用的依赖令牌及其访问来源标记。
    pub use crate::inject_wrapper::{FactoryParameter, FieldInject, Inject};
    pub use crate::metadata::{
        factory::{FactoryComponent, FactoryParameterInjection, REFLECT_METADATA_FACTORY},
        impl_bind::{InterfaceBinding, REFLECT_METADATA_BIND},
        injectable::{
            ComponentDefinition, ComponentDefinitionCallback, FieldInjection, FieldInjectionTarget,
            REFLECT_METADATA_INJECTABLE, StructComponent, component_definition,
        },
    };
    /// 仅供宏为泛型 component definition 声明其必要的服务约束。
    pub use crate::registration::injectable::Injectable;
}
