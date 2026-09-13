//! Nestrs 的 provider 注册 ABI。
//!
//! provider 是 DI 注册、选择与激活的基础单位；[`ServiceIdentifier`] 只描述一个
//! 可被请求或导出的 service token，不能再承担 provider 自身的身份。linkme 收集
//! 的是构造 [`Provider`] 的函数项，因此各 provider 仍可携带 `Vec` 形式的依赖描述。

use std::{future::Future, pin::Pin};

use linkme::distributed_slice;

use crate::{
    construction::{
        ActivationError, ConstructionContext, Constructor, ErasedService, FactoryActivationFrame,
        FactoryConstructionContext, PrepareInput,
    },
    lifetime::Lifetime,
    registration::{
        dependency::DependencyRequest, injectable::Injectable,
        service_identifier::ServiceIdentifier, service_source::ServiceSource,
        service_type::ServiceType,
    },
};

/// 一个异步 factory 激活操作的 frame-bound future。
///
/// future 持有构造输入和构造结果，调用方可以在合适的 runtime 中 await 它，再将成功的
/// concrete 输出提交到 Arena。`'frame` 同时约束 factory 参数的 `Inject` token，故它
/// 不能被伪装为 `'static` task 或 provider 输出。
pub type FactoryFuture<'frame> =
    Pin<Box<dyn Future<Output = Result<ErasedService, ActivationError>> + Send + 'frame>>;

/// 一个 cleanup hook 的 owning future。
///
/// cleanup 目前是 provider 生命周期结束时调用的无参数异步 hook；实际调度时机由未来
/// 的 scope / lifecycle runtime 决定。
pub type CleanupFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// provider 生命周期结束时调用的异步 cleanup hook。
pub type CleanupHook = fn() -> CleanupFuture;

/// 同步或异步 factory 的真实调用 ABI。
///
/// 两种 adapter 都由 `for<'frame>` 单态化，因此 factory 参数会被绑定为
/// `Inject<T, FactoryParameter<'frame>>`。同步 factory 也不能复用 class 的
/// [`Constructor`]：它同样可能错误地把参数存进返回服务。
#[derive(Debug, Clone, Copy)]
pub enum FactoryInvoker {
    Sync(FactoryConstructor),
    Async(AsyncConstructor),
}

/// 同步 factory adapter 的单态化函数签名。
pub type FactoryConstructor =
    for<'frame> fn(FactoryConstructionContext<'frame>) -> Result<ErasedService, ActivationError>;

/// 异步 factory adapter 的单态化函数签名。
pub type AsyncConstructor =
    for<'frame> fn(FactoryConstructionContext<'frame>) -> FactoryFuture<'frame>;

impl FactoryInvoker {
    /// 使用已绑定构造输入调用 factory。
    ///
    /// 这不是公开的 service-locator API。只有 core activation runtime 能同时持有真实
    /// `FactoryActivationFrame` 与 Arena 预绑定的 [`ConstructionContext`]，并保证 frame
    /// 覆盖 returned future 的整个 poll/drop 期间。
    #[allow(dead_code)] // 当前阶段尚未接线 provider activation runtime。
    pub(crate) fn invoke<'frame>(
        self,
        context: ConstructionContext,
        frame: &'frame FactoryActivationFrame,
    ) -> FactoryFuture<'frame> {
        match self {
            Self::Sync(constructor) => Box::pin(async move {
                constructor(FactoryConstructionContext::from_bound(context, frame))
            }),
            Self::Async(invoker) => invoker(FactoryConstructionContext::from_bound(context, frame)),
        }
    }
}

/// 所有可激活 provider 共享的声明属性。
#[derive(Debug, Clone, Copy)]
pub struct ProviderCommon {
    /// provider 实例的生命周期。
    pub lifetime: Lifetime,

    /// 同一 service token 存在多个候选 provider 时是否优先选用当前 provider。
    pub primary: bool,

    /// provider 的声明来源，用于冲突和激活诊断。
    pub source: ServiceSource,

    /// provider 生命周期结束时可选的异步 cleanup hook。
    pub cleanup: Option<CleanupHook>,
}

/// 为一个已闭合的 Rust 服务类型定义 provider 蓝图。
///
/// 这是宏与 runtime 之间的隐藏 ABI。它不是运行时反射：`Self` 在调用时已经是
/// `Repository<User>` 一类的闭合类型，故返回的 [`Provider`] 保留精确 constructor、
/// 依赖与 type-erasure 证明。
#[doc(hidden)]
pub trait ProviderDefinition: Injectable {
    fn provider() -> Provider
    where
        Self: Sized;
}

/// 将一个已知闭合 [`ProviderDefinition`] 转为可嵌入依赖请求的 callback 调用。
#[doc(hidden)]
pub fn provider_definition<S>() -> Provider
where
    S: ProviderDefinition,
{
    S::provider()
}

/// `#[bind]` 将 trait 请求映射到 concrete provider 时采用的 key 策略。
///
/// bind 本身不声明 key；它继承消费方请求 `dyn Trait` 时携带的 key，并据此派生待激活
/// 的 concrete provider token。这样 `#[inject(key = "red")] dyn Port` 可以匹配
/// `#[injectable(key = "red")] ConcretePort`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundKeyPolicy {
    InheritRequestedKey,
}

impl BoundKeyPolicy {
    /// 根据 trait 请求派生需要激活的 concrete token。
    pub fn concrete_identifier(
        self,
        requested: ServiceIdentifier,
        concrete_type: ServiceType,
    ) -> ServiceIdentifier {
        match self {
            Self::InheritRequestedKey => {
                ServiceIdentifier::new(requested.service_key, concrete_type)
            }
        }
    }
}

/// `#[injectable]` 注册的 class provider，对标 NestJS `useClass`。
#[derive(Debug, Clone)]
pub struct ClassProvider {
    /// provider 导出的 service token。
    pub provide: ServiceIdentifier,

    /// provider 的共享声明属性。
    pub common: ProviderCommon,

    /// 结构体字段依赖。
    pub dependencies: Vec<DependencyRequest>,

    /// 构造 concrete service 的隐藏 adapter。
    pub constructor: Constructor,
}

/// `#[factory]` 注册的 factory provider，对标 NestJS `useFactory`。
#[derive(Debug, Clone)]
pub struct FactoryProvider {
    /// provider 导出的 service token。
    pub provide: ServiceIdentifier,

    /// provider 的共享声明属性。
    pub common: ProviderCommon,

    /// factory 参数依赖。
    pub dependencies: Vec<DependencyRequest>,

    /// 同步或异步 factory 的隐藏调用 adapter。
    pub invoker: FactoryInvoker,
}

/// 将一个 concrete provider 的已提交地址投影为 trait-object 注入输入的规则。
///
/// 它不构造第二份实例，也不导出自己的 service token：resolver 应先按
/// [`BoundKeyPolicy`] 从 trait 请求派生 concrete token，再激活对应的 class 或 factory
/// provider，最后用这里的 projector 把稳定地址写入消费方槽位。
#[derive(Debug, Clone, Copy)]
pub struct TraitBinding {
    /// 被导出的 trait 类型。实际请求 key 由 `key_policy` 解释。
    pub trait_type: ServiceType,

    /// 实际需要激活的 concrete 服务类型。
    pub concrete_type: ServiceType,

    /// concrete token 如何继承 trait 请求的 key。
    pub key_policy: BoundKeyPolicy,

    /// concrete-to-trait 必选投影函数。
    pub prepare_required: PrepareInput,

    /// concrete-to-trait 可选投影函数。
    pub prepare_optional: PrepareInput,

    /// bind 声明来源。
    pub source: ServiceSource,
}

/// 一项静态 provider 注册。
///
/// 三种注册项的角色并不相同：`Class` 与 `Factory` 是实例生产者，`Bound` 是 trait 与
/// concrete 之间的投影规则。它们共用同一个 linkme 收集入口，由 registry 在归一化时
/// 按角色分区，解析路径因此不必在每次查询时重新判断变体。
#[derive(Debug, Clone)]
pub enum Provider {
    /// `#[injectable]` 注册的 class provider。
    Class(ClassProvider),

    /// `#[factory]` 注册的 factory provider。
    Factory(FactoryProvider),

    /// `#[bind]` 注册的 trait 投影规则。
    Bound(TraitBinding),
}

/// 注册项的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProviderKind {
    /// `#[injectable]` 注册的 class provider。
    Class,

    /// `#[factory]` 注册的 factory provider。
    Factory,

    /// `#[bind]` 注册的 trait 投影规则。
    Bound,
}

impl ProviderKind {
    /// 用于诊断与日志的稳定名称。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "Class",
            Self::Factory => "Factory",
            Self::Bound => "Bound",
        }
    }
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Provider {
    /// 注册项的角色。
    pub fn kind(&self) -> ProviderKind {
        match self {
            Self::Class(_) => ProviderKind::Class,
            Self::Factory(_) => ProviderKind::Factory,
            Self::Bound(_) => ProviderKind::Bound,
        }
    }

    /// 注册项的静态声明来源。
    pub fn source(&self) -> ServiceSource {
        match self {
            Self::Class(provider) => provider.common.source,
            Self::Factory(provider) => provider.common.source,
            Self::Bound(binding) => binding.source,
        }
    }

    /// 注册项导出的 service token。
    ///
    /// [`Provider::Bound`] 不导出自己的 token：它把 concrete provider 的已提交地址
    /// 投影给 trait 请求，因此这里返回 `None`。
    pub fn exported_identifier(&self) -> Option<ServiceIdentifier> {
        match self {
            Self::Class(provider) => Some(provider.provide),
            Self::Factory(provider) => Some(provider.provide),
            Self::Bound(_) => None,
        }
    }

    /// 实例生产者共享的声明属性。
    ///
    /// [`Provider::Bound`] 是投影规则而不是实例生产者，其 lifetime、primary 与 cleanup
    /// 全部继承自被绑定的 concrete provider，因此这里返回 `None`。
    pub fn common(&self) -> Option<&ProviderCommon> {
        match self {
            Self::Class(provider) => Some(&provider.common),
            Self::Factory(provider) => Some(&provider.common),
            Self::Bound(_) => None,
        }
    }
}

/// 当前链接单元内由宏或手工注册声明的 provider。
///
/// 使用函数项使每个 crate 都可在 linkme slice 中构造含 `Vec` 的 provider payload，且
/// 不要求应用 crate 直接依赖 `linkme`。
#[distributed_slice]
pub static REFLECTED_PROVIDERS: [fn() -> Provider] = [..];

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        future::Future,
        sync::Arc,
        task::{Context, Poll, Wake, Waker},
    };

    use crate::registration::{
        service_key::ServiceKey, service_source::ServiceSource, service_type::ServiceType,
    };

    struct Trait;
    struct Concrete;

    struct NoopWake;

    impl Wake for NoopWake {
        fn wake(self: Arc<Self>) {}
    }

    fn block_on<F>(future: F) -> F::Output
    where
        F: Future,
    {
        let waker = Waker::from(Arc::new(NoopWake));
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn sync_factory<'frame>(
        _context: FactoryConstructionContext<'frame>,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(()))
    }

    fn async_factory<'frame>(
        _context: FactoryConstructionContext<'frame>,
    ) -> FactoryFuture<'frame> {
        Box::pin(async { Ok(ErasedService::new(String::from("async"))) })
    }

    fn failing_factory<'frame>(
        _context: FactoryConstructionContext<'frame>,
    ) -> Result<ErasedService, ActivationError> {
        Err(ActivationError::FactoryFailed {
            provider: "failing_factory",
            provider_source: ServiceSource::new("provider.rs", 1, 1),
        })
    }

    #[test]
    fn bound_key_policy_inherits_the_trait_request_key() {
        let requested = ServiceIdentifier::new(
            Some(ServiceKey::Named("red")),
            ServiceType::create::<Trait>(),
        );
        let concrete_type = ServiceType::create::<Concrete>();

        let resolved =
            BoundKeyPolicy::InheritRequestedKey.concrete_identifier(requested, concrete_type);

        assert_eq!(resolved.service_key, Some(ServiceKey::Named("red")));
        assert_eq!(resolved.service_type, ServiceType::create::<Concrete>());
    }

    #[test]
    fn factory_invoker_preserves_its_frame_across_sync_async_and_failure_paths() {
        let frame = FactoryActivationFrame::new();

        let synchronous =
            block_on(FactoryInvoker::Sync(sync_factory).invoke(ConstructionContext::new(), &frame))
                .expect("sync factory should resolve");
        assert_eq!(synchronous.service_type(), ServiceType::create::<()>());

        let asynchronous = block_on(
            FactoryInvoker::Async(async_factory).invoke(ConstructionContext::new(), &frame),
        )
        .expect("async factory should resolve");
        assert_eq!(asynchronous.service_type(), ServiceType::create::<String>());

        let error = match block_on(
            FactoryInvoker::Sync(failing_factory).invoke(ConstructionContext::new(), &frame),
        ) {
            Err(error) => error,
            Ok(_) => panic!("failing factory should retain its activation error"),
        };
        assert!(matches!(
            error,
            ActivationError::FactoryFailed {
                provider: "failing_factory",
                ..
            }
        ));
    }
}
