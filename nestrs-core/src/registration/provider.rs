//! 生成代码提供的 Provider 声明与构造入口。
//!
//! Provider 只描述声明和构造 adapter；选择 provider、执行 factory、保存实例和生命周期
//! 语义由 graph 编译器和 runtime 协调器实现。

use std::{future::Future, pin::Pin};

use crate::{
    activation::{AsyncConstructor, ClassConstructor, FactoryConstructor},
    lifetime::ServiceLifetime,
    registration::dependency::DependencyRequest,
    service::{Injectable, ServiceIdentifier, ServiceSource},
};

/// cleanup hook 返回的独立 future，不借用已关闭 owner 的临时上下文。
pub type CleanupFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Provider 声明携带的 cleanup hook；每个成功发布的实际实例在关闭时调用一次。
pub type CleanupHook = fn() -> CleanupFuture;

/// 同步或异步 factory adapter 的函数指针。
#[derive(Debug, Clone, Copy)]
pub enum FactoryInvoker {
    Sync(FactoryConstructor),
    Async(AsyncConstructor),
}

/// 所有可激活 Provider 共享的声明属性。primary 只用于 trait 候选选择，不能覆盖
/// 同 concrete/key 的重复注册；cleanup 由运行期按成功实例执行，图编译不调用它。
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

/// injectable 声明的 class Provider。字段依赖描述与构造 adapter 分开保存，图编译
/// 只检查 dependencies，构造时才调用 constructor 准备真实字段值。
#[derive(Debug, Clone)]
pub struct ClassProvider {
    pub provide: ServiceIdentifier,
    pub common: ProviderCommon,
    pub dependencies: Vec<DependencyRequest>,
    pub constructor: ClassConstructor,
}

/// factory 声明的 Provider。函数参数同样作为显式依赖输入进入图；异步工厂与同步
/// 工厂共享图规则，区别仅保留在调用 adapter 上。
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
