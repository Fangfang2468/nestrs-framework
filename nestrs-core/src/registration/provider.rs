//! 宏生成的 provider 注册 ABI。
//!
//! Provider 只描述声明和构造 adapter；选择 provider、执行 factory、保存实例和生命周期
//! 语义由 graph 编译器和 runtime 协调器实现。

use std::{future::Future, pin::Pin};

use linkme::distributed_slice;

use crate::{
    activation::{AsyncConstructor, ClassConstructor, FactoryConstructor},
    lifetime::ServiceLifetime,
    registration::dependency::DependencyRequest,
    service::{Injectable, ServiceIdentifier, ServiceSource},
};

/// cleanup hook 的 owning future。
pub type CleanupFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Provider 元数据携带的 cleanup hook。
pub type CleanupHook = fn() -> CleanupFuture;

/// 同步或异步 factory adapter 的函数指针。
#[derive(Debug, Clone, Copy)]
pub enum FactoryInvoker {
    Sync(FactoryConstructor),
    Async(AsyncConstructor),
}

/// 所有可激活 provider 共享的声明属性。
#[derive(Debug, Clone, Copy)]
pub struct ProviderCommon {
    pub lifetime: ServiceLifetime,
    pub primary: bool,
    pub source: ServiceSource,
    pub cleanup: Option<CleanupHook>,
}

/// 为一个已闭合 Rust 服务类型定义 provider 蓝图。
///
/// 宏在泛型依赖的消费点通过该 trait 生成已闭合的 Provider 描述，不需要运行时从
/// TypeId 反推泛型实参。
#[doc(hidden)]
pub trait ProviderDefinition: Injectable {
    fn provider() -> Provider
    where
        Self: Sized;
}

/// 将已知闭合 ProviderDefinition 转为可嵌入依赖请求的 callback。
#[doc(hidden)]
pub fn provider_definition<S>() -> Provider
where
    S: ProviderDefinition,
{
    S::provider()
}

/// injectable 宏注册的 class provider。
#[derive(Debug, Clone)]
pub struct ClassProvider {
    pub provide: ServiceIdentifier,
    pub common: ProviderCommon,
    pub dependencies: Vec<DependencyRequest>,
    pub constructor: ClassConstructor,
}

/// factory 宏注册的 factory provider。
#[derive(Debug, Clone)]
pub struct FactoryProvider {
    pub provide: ServiceIdentifier,
    pub common: ProviderCommon,
    pub dependencies: Vec<DependencyRequest>,
    pub invoker: FactoryInvoker,
}

/// 一项静态 provider 注册。
#[derive(Debug, Clone)]
pub enum Provider {
    Class(ClassProvider),
    Factory(FactoryProvider),
}

/// 当前链接单元内由宏声明的 provider。
///
/// 函数项允许每个 crate 向 linkme 分布式切片提交包含 Vec 的元数据，而无需让应用
/// crate 直接依赖 linkme。
#[distributed_slice]
pub static REFLECTED_PROVIDERS: [fn() -> Provider] = [..];
