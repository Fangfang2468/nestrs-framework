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
        FactoryConstructionContext, InputPosition, PrepareInput,
    },
    lifetime::Lifetime,
    registration::{
        injectable::Injectable, service_identifier::ServiceIdentifier,
        service_source::ServiceSource, service_type::ServiceType,
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

/// 一次字段或 factory 参数的依赖请求。
///
/// 字段与函数参数都通过该结构描述，从而共享 token、可选性、构造输入位置和
/// trait-object 交付策略。`prepare_input` 仅适用于 concrete 依赖或明确的 optional
/// absence；trait object 的实际 projector 由匹配到的 [`Provider::Bound`] 提供。
#[derive(Debug, Clone, Copy)]
pub struct InjectionSpec {
    /// 依赖在原始字段或参数声明中的零基位置。
    ///
    /// 对结构体字段，该位置包含 `#[value]` / 默认字段；它只服务于稳定诊断，不等同于
    /// 构造 ABI 的输入槽位。
    pub declaration_position: usize,

    /// 依赖在构造输入中的位置。
    pub input_position: InputPosition,

    /// 查找依赖服务的 token。
    pub token: ServiceIdentifier,

    /// 缺失依赖时是否允许交付 `None`。
    pub optional: bool,

    /// 依赖诊断或元数据使用的可读标签。
    pub label: Option<&'static str>,

    /// 依赖请求的注入目标类别。
    pub target: InjectionTarget,

    /// 将 Arena 已发布的稳定地址准备为对应 `Inject<T>` 的单态化函数。
    ///
    /// 必选 trait object 和暂不支持的目标没有直接 preparer，故这里必须允许为空。
    pub prepare_input: Option<PrepareInput>,

    /// 当前注入点已单态化的泛型 provider fallback。
    ///
    /// 开放泛型的类型实参不能从 `TypeId` 反推；例如 `Repository<User>` 必须由宏在
    /// 这个调用点嵌入一个返回闭合 provider 的 callback。
    pub closed_provider: Option<ClosedProviderCallback>,
}

/// 依赖请求的目标类型形态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InjectionTarget {
    /// 普通 concrete service 类型。
    Concrete,

    /// `dyn Trait`，需要 concrete-to-trait 的 typed projector。
    TraitObject,

    /// 当前稳定地址 ABI 尚不能表示的依赖类型。
    Unsupported,
}

/// 已知闭合泛型服务转为 provider 的 callback。
pub type ClosedProviderCallback = fn() -> Provider;

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

/// 一项静态 provider 注册。
#[derive(Debug, Clone)]
pub enum Provider {
    /// `#[injectable]` 注册的 class provider，对标 NestJS `useClass`。
    Class {
        /// provider 导出的 service token。
        provide: ServiceIdentifier,

        /// provider 的共享声明属性。
        common: ProviderCommon,

        /// 结构体字段依赖。
        dependencies: Vec<InjectionSpec>,

        /// 构造 concrete service 的隐藏 adapter。
        constructor: Constructor,
    },

    /// `#[factory]` 注册的 factory provider，对标 NestJS `useFactory`。
    Factory {
        /// provider 导出的 service token。
        provide: ServiceIdentifier,

        /// provider 的共享声明属性。
        common: ProviderCommon,

        /// factory 参数依赖。
        dependencies: Vec<InjectionSpec>,

        /// 同步或异步 factory 的隐藏调用 adapter。
        invoker: FactoryInvoker,
    },

    /// 将 concrete provider 的已提交地址投影为 trait-object 注入输入。
    ///
    /// 它不构造第二份实例；resolver 应先按 [`BoundKeyPolicy`] 从 trait 请求导出
    /// concrete token，再激活对应 class 或 factory provider。
    Bound {
        /// 被导出的 trait 类型。实际请求 key 由 `key_policy` 解释。
        trait_type: ServiceType,

        /// 实际需要激活的 concrete 服务类型。
        concrete_type: ServiceType,

        /// concrete token 如何继承 trait 请求的 key。
        key_policy: BoundKeyPolicy,

        /// concrete-to-trait 必选投影函数。
        prepare_required: PrepareInput,

        /// concrete-to-trait 可选投影函数。
        prepare_optional: PrepareInput,

        /// bind 声明来源。
        source: ServiceSource,
    },

    /// 将一个 token 重定向到另一个 provider token，对标 NestJS `useExisting`。
    Alias {
        /// alias 导出的 token。
        provide: ServiceIdentifier,

        /// 实际目标 token。
        target: ServiceIdentifier,

        /// alias 声明来源。
        source: ServiceSource,
    },
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
