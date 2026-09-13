//! Nestrs 的 provider 注册 ABI。
//!
//! provider 只表示「能为某个 token 产出实例」的注册：`#[injectable]` 的 class provider
//! 与 `#[factory]` 的 factory provider；[`ServiceIdentifier`] 只描述一个可被请求或
//! 导出的 service token，不能承担 provider 自身的身份。
//!
//! trait 与 concrete 之间的投影规则不是 provider，它由
//! [`crate::registration::binding::TraitBinding`] 表达并单独收集。linkme 收集的是构造
//! [`Provider`] 的函数项，因此各 provider 仍可携带 `Vec` 形式的依赖描述。

use std::{future::Future, pin::Pin};

use linkme::distributed_slice;

use crate::{
    construction::{
        ActivationError, ConstructionContext, Constructor, ErasedService, FactoryActivationFrame,
        FactoryConstructionContext,
    },
    lifetime::Lifetime,
    registration::{
        dependency::DependencyRequest, injectable::Injectable,
        service_identifier::ServiceIdentifier, service_source::ServiceSource,
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

/// 一项静态 provider 注册。
///
/// 两个变体都是实例生产者，差异只在实例如何被构造：结构体字段构造或工厂函数调用。
/// 解析路径按声明种类分派，与 NestJS 的 provider 解析方式一致。
#[derive(Debug, Clone)]
pub enum Provider {
    /// `#[injectable]` 注册的 class provider。
    Class(ClassProvider),

    /// `#[factory]` 注册的 factory provider。
    Factory(FactoryProvider),
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

    use crate::registration::{service_source::ServiceSource, service_type::ServiceType};

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
