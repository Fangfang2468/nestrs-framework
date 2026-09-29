//! 查询与绑定宏在具体类型调用点提供的静态闭合类型探测。
//!
//! 已闭合且实现 ProviderDefinition 的类型携带单态化 callback；factory-only 类型和
//! trait 查询只记录类型，不要求一个不存在的 ProviderDefinition 实现。根描述在链接时
//! 收集，在静态图编译期间消费，不提供运行期扩图入口。

use linkme::distributed_slice;
use std::marker::PhantomData;

use crate::{
    registration::{
        dependency::ClosedProviderCallback,
        provider::{ProviderDefinition, provider_definition},
    },
    service::{Injectable, ServiceSource, ServiceType},
};

/// 查询宏生成的根描述。没有物化 callback 的查询可以正常表示一个缺席的可选服务。
#[derive(Debug, Clone, Copy)]
pub struct RootDeclaration {
    pub service_type: ServiceType,
    pub materialize: Option<ClosedProviderCallback>,
    pub source: ServiceSource,
}

/// 重复查询同一闭合类型是幂等的；图编译器优先采用存在的物化 callback。
#[distributed_slice]
pub static REFLECTED_ROOTS: [fn() -> RootDeclaration] = [..];

/// Compiler-discovered closed blueprints. Unlike roots these are passive:
/// graph compilation only calls a blueprint demanded by a root or dependency.
#[distributed_slice]
pub static REFLECTED_BLUEPRINTS: [fn() -> RootDeclaration] = [..];

/// A type-level dependency path keeps a producer's private field types behind
/// an ordinary, compiler-checked callback. The const is forwarded at compile
/// time; looking up a deep path never recursively walks it at runtime.
#[doc(hidden)]
pub struct DependencySlot<const SLOT: usize>;

#[doc(hidden)]
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

/// 查询与绑定宏专用的 autoref 探测令牌。
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
mod tests {
    use super::*;
    use crate::registration::provider::Provider;

    struct Defined<T>(PhantomData<T>);
    impl<T: Send + Sync + 'static> ProviderDefinition for Defined<T> {
        fn provider() -> Provider {
            panic!("discovering a callback must never execute it")
        }
    }
    struct FactoryOnly;
    struct GenericFactoryOnly<T>(PhantomData<T>);
    trait Port: Send + Sync {}

    #[test]
    fn concrete_site_probe_finds_definitions_through_type_aliases() {
        type Alias = Defined<u32>;
        assert!(
            (&&Probe::<Defined<u32>>::new())
                .provider_callback()
                .is_some()
        );
        assert!((&&Probe::<Alias>::new()).provider_callback().is_some());
    }

    #[test]
    #[allow(clippy::needless_borrow)] // Keep the exact macro autoref protocol for fallback types.
    fn factory_only_generic_and_trait_queries_use_the_unbounded_fallback() {
        type FactoryAlias = GenericFactoryOnly<u32>;
        type TraitAlias = dyn Port;
        assert!(
            (&&Probe::<FactoryOnly>::new())
                .provider_callback()
                .is_none()
        );
        assert!(
            (&&Probe::<GenericFactoryOnly<u32>>::new())
                .provider_callback()
                .is_none()
        );
        assert!(
            (&&Probe::<FactoryAlias>::new())
                .provider_callback()
                .is_none()
        );
        assert!((&&Probe::<dyn Port>::new()).provider_callback().is_none());
        assert!((&&Probe::<TraitAlias>::new()).provider_callback().is_none());
    }
}
