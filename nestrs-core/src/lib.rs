mod activation;
mod facade;
mod lifetime;
mod registration;
mod service;

pub use facade::{BuildError, ResolveError, ServiceProvider, ServiceScope, ShutdownError};
pub use lifetime::ServiceLifetime;
pub use service::ServiceKey;

#[doc(hidden)]
pub mod __private {
    /// 仅供宏生成的分布式注册静态项使用的 `linkme` crate 重导出。
    ///
    /// 下游应用无需也不应为了 DI 注册而直接依赖此名称。
    pub use linkme;

    /// 仅供宏展开引用的只读字段注入 token。
    pub use crate::activation::Injection;
    /// 仅供宏生成构造 adapter 使用的隐藏 ABI。
    pub use crate::activation::{
        ActivationError, ConstructionContext, FactoryConstructionContext, InputPosition,
        PrepareInput, prepare_bound_optional, prepare_bound_required, prepare_optional,
        prepare_optional_absent, prepare_required,
    };
    /// 仅供宏生成构造 adapter 使用的 type-erased service ABI。
    pub use crate::activation::{Constructor, ErasedService, ErasedServiceRef};
    /// 宏生成 provider metadata 所需的服务生命周期枚举。
    pub use crate::lifetime::ServiceLifetime;
    /// 仅供宏写入和读取的 trait 绑定注册 ABI。
    pub use crate::registration::binding::{BoundKeyPolicy, REFLECTED_BINDINGS, TraitBinding};
    /// 仅供宏写入和读取的依赖请求 ABI。
    pub use crate::registration::dependency::{
        ClosedProviderCallback, Delivery, DependencyRequest, ProviderSource,
    };
    /// 仅供宏写入和读取的实例 provider 注册 ABI。
    pub use crate::registration::provider::{
        AsyncConstructor, ClassProvider, CleanupFuture, CleanupHook, FactoryConstructor,
        FactoryFuture, FactoryInvoker, FactoryProvider, Provider, ProviderCommon,
        ProviderDefinition, REFLECTED_PROVIDERS, provider_definition,
    };
    /// 仅供宏为泛型 provider definition 声明其必要的服务约束。
    pub use crate::service::Injectable;
    /// 宏生成注册 metadata 所需的服务 token ABI。
    pub use crate::service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType};
}
