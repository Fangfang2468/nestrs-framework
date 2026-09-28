//! 静态注册、先验证后激活的 DI 门面。

use std::{
    fmt,
    num::NonZeroUsize,
    sync::{Arc, Mutex},
};

use crate::{
    ServiceKey, ServiceLifetime,
    activation::InputSlot,
    graph::{GraphCompiler, ValidatedGraph},
    runtime::{Owner, Runtime},
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};

/// 提前初始化的范围。Eager 只预热 Singleton 及其必要依赖。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InitializationMode {
    #[default]
    Lazy,
    Eager,
}

/// 所有 scope 共享 root 的构造并发上限。
#[derive(Debug, Clone)]
pub struct ServiceProviderOptions {
    pub initialization: InitializationMode,
    pub max_concurrent_activations: NonZeroUsize,
}

impl Default for ServiceProviderOptions {
    fn default() -> Self {
        Self {
            initialization: InitializationMode::Lazy,
            max_concurrent_activations: NonZeroUsize::new(32).unwrap(),
        }
    }
}

/// 运行环境或 Eager 初始化失败。静态图错误直接 panic。
#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("构建服务容器需要当前 Tokio runtime")]
    RuntimeUnavailable,
    #[error("服务容器预热失败: {error}; 关闭结果: {dispose_error:?}")]
    Initialization {
        #[source]
        error: ResolveError,
        dispose_error: Option<DisposeError>,
    },
}

#[derive(Debug)]
struct ResolveFailure {
    detail: Arc<str>,
    path: Arc<Mutex<Vec<FailureFrame>>>,
    tail: Option<usize>,
}

#[derive(Debug)]
struct FailureFrame {
    identifier: ServiceIdentifier,
    source: ServiceSource,
    parent: Option<usize>,
}

/// 获取或激活失败，保留失败原因和带 key、源码位置的依赖路径。
///
/// Singleton/Scoped 的失败被缓存，后续调用共享原始失败记录。
#[derive(Debug, Clone)]
pub struct ResolveError(Arc<ResolveFailure>);

impl ResolveError {
    pub(crate) fn new(detail: String) -> Self {
        Self(Arc::new(ResolveFailure {
            detail: detail.into(),
            path: Arc::new(Mutex::new(Vec::new())),
            tail: None,
        }))
    }

    pub(crate) fn closed() -> Self {
        Self::new("服务 owner 已关闭或正在关闭".to_owned())
    }

    pub(crate) fn construction(
        identifier: &ServiceIdentifier,
        source: ServiceSource,
        detail: String,
    ) -> Self {
        Self::dependency(identifier, source, Self::new(detail))
    }

    pub(crate) fn dependency(
        identifier: &ServiceIdentifier,
        source: ServiceSource,
        dependency: Self,
    ) -> Self {
        let mut frames = dependency
            .0
            .path
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let tail = frames.len();
        frames.push(FailureFrame {
            identifier: identifier.clone(),
            source,
            parent: dependency.0.tail,
        });
        drop(frames);
        Self(Arc::new(ResolveFailure {
            detail: dependency.0.detail.clone(),
            path: dependency.0.path.clone(),
            tail: Some(tail),
        }))
    }
}

impl fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "服务解析失败: {}", self.0.detail)?;
        let frames = self
            .0
            .path
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut current = self.0.tail;
        while let Some(index) = current {
            let FailureFrame {
                identifier,
                source,
                parent,
            } = &frames[index];
            current = *parent;
            write!(
                formatter,
                "\n  {} key={:?} ({}:{}:{})",
                identifier.service_type.name,
                identifier.service_key,
                source.file,
                source.line,
                source.column
            )?;
        }
        Ok(())
    }
}
impl std::error::Error for ResolveError {}

/// 所有 cleanup 都处理完成后汇总的关闭错误。
#[derive(Debug, Clone, thiserror::Error)]
#[error("服务容器关闭失败: {failures:?}")]
pub struct DisposeError {
    failures: Arc<Vec<String>>,
}
impl DisposeError {
    pub(crate) fn new(failures: Vec<String>) -> Self {
        Self {
            failures: Arc::new(failures),
        }
    }

    /// 每个失败 cleanup 的诊断。其余实例仍会继续清理。
    pub fn failures(&self) -> &[String] {
        &self.failures
    }
}

/// 应用级服务 owner。使用查询宏获取服务，结果引用借用实际 owner。
///
/// 服务只可通过宏静态声明，构建后图被冻结。Drop 非阻塞地发起关闭；需要等待所有
/// cleanup 时必须调用 [`Self::dispose_async`]。
#[must_use]
pub struct ServiceProvider {
    graph: Arc<ValidatedGraph>,
    runtime: Arc<Runtime>,
    owner: Arc<Owner>,
}

impl ServiceProvider {
    pub async fn build() -> Result<Self, BuildError> {
        Self::build_with_options(ServiceProviderOptions::default()).await
    }

    /// 在执行任何构造函数之前完整验证图；结构错误立即 panic。
    ///
    pub async fn build_with_options(options: ServiceProviderOptions) -> Result<Self, BuildError> {
        let graph = Arc::new(
            GraphCompiler::compile_static()
                .unwrap_or_else(|error| panic!("DI 依赖图验证失败: {error}")),
        );
        tokio::runtime::Handle::try_current().map_err(|_| BuildError::RuntimeUnavailable)?;
        let (runtime, owner) =
            Runtime::start(graph.clone(), options.max_concurrent_activations.get());
        let provider = Self {
            graph,
            runtime,
            owner,
        };
        if options.initialization == InitializationMode::Eager
            && let Err(error) = provider
                .runtime
                .warm_up(&provider.owner, ServiceLifetime::Singleton)
                .await
        {
            let dispose_error = provider.runtime.close(&provider.owner).await.err();
            return Err(BuildError::Initialization {
                error,
                dispose_error,
            });
        }
        Ok(provider)
    }

    fn view(&self) -> ServiceProviderRef<'_> {
        ServiceProviderRef {
            graph: &self.graph,
            runtime: &self.runtime,
            owner: &self.owner,
        }
    }

    /// 同步建立独立 Scoped owner，不执行任何服务构造。
    pub fn create_scope(&self) -> ServiceScope<'_> {
        ServiceScope {
            provider: self,
            owner: self.runtime.create_scope(),
        }
    }

    /// 消费 owner，等待已接受构造、scope 关闭及本 owner 的所有 cleanup。
    /// 取消等待不会取消关闭；不返回的 factory/cleanup 会使关闭持续等待。
    pub async fn dispose_async(self) -> Result<(), DisposeError> {
        self.runtime.close(&self.owner).await
    }
}

impl Drop for ServiceProvider {
    fn drop(&mut self) {
        self.runtime.request_close(&self.owner);
    }
}

/// Scoped 实例的 owner，借用 root 并隔离其他 scope 的缓存。
#[must_use]
pub struct ServiceScope<'provider> {
    provider: &'provider ServiceProvider,
    owner: Arc<Owner>,
}
impl ServiceScope<'_> {
    pub fn service_provider(&self) -> ServiceProviderRef<'_> {
        ServiceProviderRef {
            graph: &self.provider.graph,
            runtime: &self.provider.runtime,
            owner: &self.owner,
        }
    }
    pub async fn warm_up(&self) -> Result<(), ResolveError> {
        self.provider
            .runtime
            .warm_up(&self.owner, ServiceLifetime::Scoped)
            .await
    }
    pub async fn dispose_async(self) -> Result<(), DisposeError> {
        self.provider.runtime.close(&self.owner).await
    }
}
impl Drop for ServiceScope<'_> {
    fn drop(&mut self) {
        self.provider.runtime.request_close(&self.owner);
    }
}

/// 传给查询宏的轻量视图；临时视图不会缩短结果引用的有效期。
#[derive(Clone, Copy)]
pub struct ServiceProviderRef<'owner> {
    graph: &'owner ValidatedGraph,
    runtime: &'owner Arc<Runtime>,
    owner: &'owner Arc<Owner>,
}
impl<'owner> ServiceProviderRef<'owner> {
    async fn required<T: ?Sized + Send + Sync + 'static>(
        self,
        key: Option<ServiceKey>,
    ) -> Result<&'owner T, ResolveError> {
        self.query::<T>(key.clone()).await?.ok_or_else(|| {
            ResolveError::new(format!(
                "服务未注册: {} key={key:?}",
                std::any::type_name::<T>()
            ))
        })
    }

    async fn query<T: ?Sized + Send + Sync + 'static>(
        self,
        key: Option<ServiceKey>,
    ) -> Result<Option<&'owner T>, ResolveError> {
        if self.owner.is_closed() {
            return Err(ResolveError::closed());
        }
        let identifier = ServiceIdentifier::new(key, ServiceType::create::<T>());
        let Some(route) = self.graph.routes.get(&identifier) else {
            return Ok(None);
        };
        let lease = self.runtime.resolve(self.owner, route.provider).await?;
        let pointer = if let Some(project) = route.projection {
            let prepared = project(InputSlot::new(0), Some(lease.erased_ref()))
                .map_err(|error| ResolveError::new(error.to_string()))?;
            let token = prepared
                .into_required::<T>(InputSlot::new(0))
                .map_err(|error| ResolveError::new(error.to_string()))?;
            if !token.lease().ptr_eq(&lease) {
                return Err(ResolveError::new(
                    "trait 投影返回了不同实例的注入令牌".to_owned(),
                ));
            }
            token.into_ptr()
        } else {
            lease.pointer::<T>().ok_or_else(|| {
                ResolveError::new(format!(
                    "构造结果类型不匹配: {}",
                    identifier.service_type.name
                ))
            })?
        };
        // SAFETY: runtime.resolve publishes a lease in the actual Owner's journal before
        // completing this request. Owner retains that journal even after runtime shutdown.
        // Its borrowing facade cannot be consumed/dropped while this reference is used.
        // Projection above validates T and obtains its exact (possibly wide) typed address.
        Ok(Some(unsafe { pointer.as_ref() }))
    }
}

/// Macro-only query normalization. Its return value borrows the actual owner, not a temporary
/// view or this receiver. This is hidden expansion ABI, not a supported service-query method.
#[doc(hidden)]
pub trait QueryTarget<'owner> {
    fn __nestrs_query_view(self) -> ServiceProviderRef<'owner>;
}

impl<'owner> QueryTarget<'owner> for &'owner ServiceProvider {
    fn __nestrs_query_view(self) -> ServiceProviderRef<'owner> {
        self.view()
    }
}

impl<'owner> QueryTarget<'owner> for &ServiceProviderRef<'owner> {
    fn __nestrs_query_view(self) -> ServiceProviderRef<'owner> {
        *self
    }
}

/// Expansion bridge. Must be public because exported macros expand in downstream crates.
#[doc(hidden)]
pub async fn query_required<'owner, T: ?Sized + Send + Sync + 'static>(
    view: ServiceProviderRef<'owner>,
    key: Option<ServiceKey>,
) -> Result<&'owner T, ResolveError> {
    view.required::<T>(key).await
}

/// Expansion bridge preserving optional-missing versus initialization-failure semantics.
#[doc(hidden)]
pub async fn query_optional<'owner, T: ?Sized + Send + Sync + 'static>(
    view: ServiceProviderRef<'owner>,
    key: Option<ServiceKey>,
) -> Result<Option<&'owner T>, ResolveError> {
    view.query::<T>(key).await
}

#[cfg(test)]
mod error_path_tests;
