mod activation;
mod facade;
mod graph;
mod lifetime;
mod query;
mod registration;
mod runtime;
mod service;

pub use facade::{
    BuildError, DisposeError, InitializationMode, ResolveError, ServiceProvider,
    ServiceProviderOptions, ServiceProviderRef, ServiceScope,
};
pub use lifetime::ServiceLifetime;
pub use service::ServiceKey;

#[doc(hidden)]
pub mod __private {
    /// Versioned diagnostic bridge for `cargo nestrs graph`. Only metadata
    /// callbacks run; no runtime, service constructor or cleanup is started.
    pub fn dependency_graph_json() -> Result<String, String> {
        crate::graph::GraphCompiler::compile_static()
            .map(|graph| crate::graph::snapshot(&graph).to_string())
            .map_err(|error| format!("DI 依赖图验证失败: {error}"))
    }

    /// 仅供宏生成的分布式注册静态项使用的 `linkme` crate 重导出。
    ///
    /// 下游应用无需也不应为了 DI 注册而直接依赖此名称。
    pub use linkme;

    /// 仅供宏展开引用的只读字段注入 token。
    pub use crate::activation::Injection;
    /// 仅供宏生成 factory adapter 使用的隐藏 ABI。
    pub use crate::activation::{AsyncConstructor, FactoryConstructor, FactoryFuture};
    /// 仅供宏生成构造 adapter 使用的 type-erased service ABI。
    pub use crate::activation::{ClassConstructor, ErasedService, ErasedServiceRef};
    /// 仅供宏生成构造 adapter 使用的隐藏 ABI。
    pub use crate::activation::{
        ConstructionError, ConstructionInputs, FactoryInputs, InputPreparer, InputSlot,
        PreparedInput, prepare_bound_optional, prepare_bound_required, prepare_optional,
        prepare_optional_absent, prepare_required,
    };
    pub use crate::facade::{QueryTarget, query_optional, query_required};
    /// 宏生成 provider metadata 所需的服务生命周期枚举。
    pub use crate::lifetime::ServiceLifetime;
    /// 仅供宏写入和读取的 trait 绑定注册 ABI。
    pub use crate::registration::binding::{
        BoundKeyPolicy, REFLECTED_AUTOMATIC_BINDINGS, REFLECTED_BINDINGS, TraitBinding,
    };
    /// Type-bearing markers for the versioned compiler adapter; not a user API.
    pub use crate::registration::compiler::{
        CompilerKey, compiler_automatic_binding, compiler_binding, compiler_blueprint,
        compiler_blueprint_path, compiler_dependency, compiler_provider, compiler_request,
    };
    /// 仅供宏写入和读取的依赖请求 ABI。
    pub use crate::registration::dependency::{
        ClosedProviderCallback, Delivery, DependencyRequest, ProviderSource,
    };
    /// 仅供宏写入和读取的实例 provider 注册 ABI。
    pub use crate::registration::provider::{
        ClassProvider, CleanupFuture, CleanupHook, FactoryInvoker, FactoryProvider, Provider,
        ProviderCommon, ProviderDefinition, REFLECTED_PROVIDERS, provider_definition,
    };
    pub use crate::registration::root::{
        DependencyPath, DependencySlot, Probe, ProbeProvider, REFLECTED_BLUEPRINTS,
        REFLECTED_ROOTS, RootDeclaration,
    };
    /// 仅供宏为泛型 provider definition 声明其必要的服务约束。
    pub use crate::service::Injectable;
    /// 宏生成注册 metadata 所需的服务 token ABI。
    pub use crate::service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType};
}
