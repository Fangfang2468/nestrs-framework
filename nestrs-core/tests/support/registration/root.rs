//! 声明 adapter 与测试 oracle 共用的静态闭合类型探测协议。
//!
//! 已闭合且实现 ProviderDefinition 的类型携带单态化 callback；factory-only 类型和
//! trait 查询只记录类型，不要求一个不存在的 ProviderDefinition 实现。普通查询根已由
//! 编译器分析方法与闭合调用摘要；保留的 typed 描述用于声明能力和测试快照，不提供
//! 运行期扩图入口，公开查询宏也不再生成这些回调。

use std::marker::PhantomData;

use crate::{
    registration::{
        dependency::ClosedProviderCallback,
        provider::{ProviderDefinition, provider_definition},
    },
    service::{Injectable, ServiceSource, ServiceType},
};

/// 闭合类型的描述锚点。没有物化 callback 时，也可以表示缺席的可选服务。
#[derive(Debug, Clone, Copy)]
pub struct RootDeclaration {
    pub service_type: ServiceType,
    pub materialize: Option<ClosedProviderCallback>,
    pub source: ServiceSource,
}

/// 类型级依赖路径把生产 crate 的私有字段类型留在普通、经编译器检查的回调之后。
/// SLOT 在编译期传递；深层路径不需要运行期递归查找，也不要求公开业务私有类型。
#[doc(hidden)]
pub struct DependencySlot<const SLOT: usize>;

#[doc(hidden)]
/// 路径最终提供已闭合 Provider 的根描述，供下游完整入口汇总被动蓝图。
pub trait DependencyPath<Path> {
    const BLUEPRINT: fn() -> RootDeclaration;
}

impl<T: ProviderDefinition> DependencyPath<()> for T {
    const BLUEPRINT: fn() -> RootDeclaration = || RootDeclaration {
        service_type: ServiceType::create::<T>(),
        materialize: Some(provider_definition::<T>),
        source: ServiceSource::new(file!(), line!(), column!()),
    };
}

/// 声明 adapter 的 autoref 探测令牌，不承担普通查询方法的根收集。
///
/// 必须在宏展开的具体类型处调用 `(&&Probe::<T>::new()).provider_callback()`；
/// 不能把方法解析移入没有 ProviderDefinition 约束的泛型 helper，因为单态化不会
/// 重新选择已经确定的 fallback。此类型不创建或持有任何服务。
#[doc(hidden)]
pub struct Probe<T: ?Sized> {
    marker: PhantomData<fn(&T)>,
}

impl<T: ?Sized> Probe<T> {
    pub const fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl<T: ?Sized> Default for Probe<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// 宏应将此 trait 以匿名导入放到调用点，避免依赖调用方的 use 列表。
#[doc(hidden)]
pub trait ProbeProvider {
    fn provider_callback(self) -> Option<ClosedProviderCallback>;
}

impl<T: ProviderDefinition> ProbeProvider for &&Probe<T> {
    fn provider_callback(self) -> Option<ClosedProviderCallback> {
        Some(provider_definition::<T>)
    }
}

impl<T: Injectable + ?Sized> ProbeProvider for &Probe<T> {
    fn provider_callback(self) -> Option<ClosedProviderCallback> {
        None
    }
}

#[cfg(test)]
#[path = "../../unit/registration/root.rs"]
mod tests;

/// 编译器为普通查询方法汇总根时调用的类型化入口。没有 ProviderDefinition 的类型
/// （例如 trait 或仅由 factory 生产的类型）同样可以贡献查询根，不人为增加用户约束。
#[inline(never)]
pub fn query_root<T: Injectable + ?Sized>() -> RootDeclaration {
    RootDeclaration {
        service_type: ServiceType::create::<T>(),
        materialize: None,
        source: ServiceSource::new("<compiler query>", 0, 0),
    }
}

/// 与 query_root 使用相同描述布局；是否具备蓝图由 rustc 的真实 trait 求解确定。
/// 该入口只返回函数地址，编译期和收集期均不会构造 T。
#[inline(never)]
pub fn query_provider_root<T: ProviderDefinition>() -> RootDeclaration {
    RootDeclaration {
        service_type: ServiceType::create::<T>(),
        materialize: Some(provider_definition::<T>),
        source: ServiceSource::new("<compiler query>", 0, 0),
    }
}
