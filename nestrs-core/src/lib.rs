pub mod arena;
mod construction;
mod inject_wrapper;
pub mod lifetime;
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
        ActivationError, ConstructionContext, Constructor, ErasedService,
        FactoryConstructionContext, InputPosition, PrepareInput, prepare_bound_optional,
        prepare_bound_required, prepare_optional, prepare_optional_absent, prepare_required,
    };
    /// 仅供宏展开引用的依赖令牌及其访问来源标记。
    pub use crate::inject_wrapper::{FactoryParameter, FieldInject, Inject};
    /// 仅供宏为泛型 provider definition 声明其必要的服务约束。
    pub use crate::registration::injectable::Injectable;
    /// 仅供宏写入和读取的依赖请求 ABI。
    pub use crate::registration::dependency::{
        ClosedProviderCallback, DependencyRequest, Delivery, ProviderSource,
    };
    /// 仅供宏写入和读取的实例 provider 注册 ABI。
    pub use crate::registration::provider::{
        AsyncConstructor, ClassProvider, CleanupFuture, CleanupHook, FactoryConstructor,
        FactoryFuture, FactoryInvoker, FactoryProvider, Provider, ProviderCommon,
        ProviderDefinition, REFLECTED_PROVIDERS, provider_definition,
    };
    /// 仅供宏写入和读取的 trait 绑定注册 ABI。
    pub use crate::registration::binding::{
        BoundKeyPolicy, REFLECTED_BINDINGS, TraitBinding,
    };
}
