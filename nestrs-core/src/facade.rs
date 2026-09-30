//! 静态注册、先验证后激活的 DI 门面。
//!
//! 这里仅组合图、运行时命令和用户可借用的 owner。错误链、调度状态及实例释放
//! 分别由 error、runtime、activation 管理，门面不保存第二套生命周期状态。
//! 返回 `&T` 的安全边界保留在本模块：运行时先把实例发布到实际 owner 的 journal，
//! 查询才将验证后的 typed pointer 恢复为借用该 owner 的引用。

use std::sync::Arc;

use crate::{
    InitializationMode, ServiceKey, ServiceLifetime, ServiceProviderOptions,
    activation::InputSlot,
    error::{BuildError, DisposeError, ResolveError},
    graph::{GraphCompiler, ValidatedGraph},
    runtime::{Owner, Runtime},
    service::{ServiceIdentifier, ServiceType},
};

/// 应用级服务 owner。使用查询宏获取服务，结果引用借用实际 owner。
///
/// 服务只可通过宏静态声明，构建后图被冻结。Drop 非阻塞地发起关闭；需要等待所有
/// cleanup 时必须调用 [`Self::dispose_async`]。
#[must_use]
pub struct ServiceProvider {
    graph: Arc<ValidatedGraph>,
    runtime: Arc<Runtime>,
    // Owner 的 Drop 统一发送非阻塞关闭请求。门面不再重复实现同一兜底行为；
    // 即使 dispose_async 的 future 未完成就被丢弃，持有的 owner 也会负责发起关闭。
    owner: Arc<Owner>,
}

impl ServiceProvider {
    /// 使用 Lazy 与 32 个构造名额的默认选项建立容器。
    pub async fn build() -> Result<Self, BuildError> {
        Self::build_with_options(ServiceProviderOptions::default()).await
    }

    /// 在执行任何构造函数之前完整验证图；结构错误立即 panic。
    pub async fn build_with_options(options: ServiceProviderOptions) -> Result<Self, BuildError> {
        // 先核查全部结构，再启动协调器；即使没有 Tokio runtime，非法图也不会
        // 因为环境检查而绕过诊断，更不会在验证完成前执行任何服务构造。
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
            // 容器尚未交付给调用者。预热失败时仍等待已经接受的工作与清理，
            // 同时保留初始化错误和清理错误，不能把部分成功实例直接遗弃。
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

/// Scoped 实例的 owner，借用 root 并隔离其他 scope 的缓存。
#[must_use]
pub struct ServiceScope<'provider> {
    provider: &'provider ServiceProvider,
    owner: Arc<Owner>,
}
impl ServiceScope<'_> {
    /// 创建轻量查询视图；结果引用绑定当前 scope，而不是这个临时视图。
    pub fn service_provider(&self) -> ServiceProviderRef<'_> {
        ServiceProviderRef {
            graph: &self.provider.graph,
            runtime: &self.provider.runtime,
            owner: &self.owner,
        }
    }
    /// 提交当前 scope 的全部 Scoped 根及必要依赖；保持既有生命周期与并发上限。
    pub async fn warm_up(&self) -> Result<(), ResolveError> {
        self.provider
            .runtime
            .warm_up(&self.owner, ServiceLifetime::Scoped)
            .await
    }
    /// 消费当前 scope，等待已接受任务及本 scope 的串行 cleanup 完成。
    /// 查询结果仍被使用时，Rust 借用检查会拒绝消费这个 scope。
    pub async fn dispose_async(self) -> Result<(), DisposeError> {
        self.provider.runtime.close(&self.owner).await
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
        // concrete 查询复用实例保存的准确地址；trait 查询复用构图时选中的 typed
        // 投影。两条路径都必须验证类型，trait 路径还核对投影没有替换实例所有者。
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
        // SAFETY: runtime.resolve 在完成请求前已将强 lease 发布到实际 owner 的 journal。
        // journal 的存活期跟随 owner，不依赖 Tokio 协调任务仍在运行；返回引用被使用时，
        // 借用检查禁止消费/丢弃该 owner 的门面。上方同时核对 T 和准确的（可能为宽）地址。
        Ok(Some(unsafe { pointer.as_ref() }))
    }
}

/// 查询宏统一 root 与视图接收者的内部协议。
/// 返回值借用真正的 owner，不借用这个临时接收者；业务源码不能直接使用该私有接口。
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

/// 必选查询的生成代码入口，所在模块保持私有，仅由编译器授权的查询宏访问。
#[doc(hidden)]
pub async fn query_required<'owner, T: ?Sized + Send + Sync + 'static>(
    view: ServiceProviderRef<'owner>,
    key: Option<ServiceKey>,
) -> Result<&'owner T, ResolveError> {
    view.required::<T>(key).await
}

/// 可选查询的生成代码入口；只有未注册返回 None，初始化等其他失败仍返回错误。
#[doc(hidden)]
pub async fn query_optional<'owner, T: ?Sized + Send + Sync + 'static>(
    view: ServiceProviderRef<'owner>,
    key: Option<ServiceKey>,
) -> Result<Option<&'owner T>, ResolveError> {
    view.query::<T>(key).await
}
