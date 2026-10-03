//! 生成代码提供的 Provider 声明与构造入口。
//!
//! Provider 只描述声明和构造 adapter；选择 provider、执行 factory、保存实例和生命周期
//! 语义由 graph 编译器和 runtime 协调器实现。

use crate::{
    activation::ClassConstructor,
    lifetime::ServiceLifetime,
    registration::dependency::DependencyRequest,
    service::{Injectable, ServiceIdentifier, ServiceSource},
};

pub use crate::activation::adapter::{CleanupFuture, CleanupHook, FactoryInvoker};

/// 所有可激活 Provider 共享的声明属性。primary 只用于 trait 候选选择，不能覆盖
/// 同 concrete/key 的重复注册；cleanup 由运行期按成功实例执行，图编译不调用它。
#[derive(Debug, Clone, Copy)]
pub struct ProviderCommon {
    pub lifetime: ServiceLifetime,
    pub primary: bool,
    /// 服务声明级初始化覆盖：None 继承预热默认值，true 延迟，false 主动预热。
    ///
    /// 只影响是否作为独立预热入口，不改变普通依赖边的构造要求。root 的默认值来自
    /// build 选项；显式 scope.warm_up 默认预热 Scoped。Transient 不作为预热入口，
    /// 因而该标记不改变其逐消费构造语义，也不改变任何字段的 Injection 包装。
    pub lazy: Option<bool>,
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

// 参考算法只在完成候选选择后丢弃 primary；生产计划根本没有这个字段。
impl From<ProviderCommon> for crate::graph::NodePolicy {
    fn from(common: ProviderCommon) -> Self {
        Self {
            lifetime: common.lifetime,
            lazy: common.lazy,
            source: common.source,
            cleanup: common.cleanup,
        }
    }
}
