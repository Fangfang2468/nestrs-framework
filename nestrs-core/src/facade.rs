//! 装载编译计划并按需激活服务的 DI 门面。
//!
//! 这里仅组合图、运行时命令和用户可借用的 owner。错误链、调度状态及实例释放
//! 分别由 error、runtime、activation 管理，门面不保存第二套生命周期状态。
//! 返回 `&T` 的安全边界保留在本模块：运行时先把实例发布到实际 owner 的 journal，
//! 查询才将验证后的 typed pointer 恢复为借用该 owner 的引用。

use std::sync::Arc;

use crate::{
    InitializationMode, ServiceKey, ServiceProviderOptions, ServiceScopeOptions,
    activation::{InputSlot, ProjectionTarget},
    error::{BuildError, DisposeError, ResolveError, ScopeBuildError},
    graph::{ValidatedGraph, plan::CompiledApplication},
    runtime::{Owner, Runtime},
    service::{ServiceIdentifier, ServiceType},
};

/// 应用级服务 owner。使用普通异步方法获取服务，结果引用借用实际 owner。
///
/// 工具链收集服务声明与查询根，在编译期验证并冻结图。Drop 非阻塞地发起关闭；需要等待所有
/// cleanup 时必须调用 [`Self::dispose_async`]。
#[must_use]
pub struct ServiceProvider {
    /// 当前入口共享的不可变执行计划。
    graph: Arc<ValidatedGraph>,

    /// 用于提交当前 owner 查询与关闭的共享协调器句柄。
    runtime: Arc<Runtime>,

    // Owner 的 Drop 统一发送非阻塞关闭请求。门面不再重复实现同一兜底行为；
    // 即使 dispose_async 的 future 未完成就被丢弃，持有的 owner 也会负责发起关闭。
    /// 本次操作所属的实际 root 或 scope。
    owner: Arc<Owner>,

    /// 当前容器的 scope 创建默认值，独立于 root 初始化策略。
    scope_initialization: InitializationMode,
}

impl ServiceProvider {
    /// 建立独立容器，完成选中的 Singleton 初始化后交付。
    ///
    /// `None` 使用工具链固化的入口项目配置；未配置时 root / scope 均为 Lazy，并发为 32。
    /// `Some(options)` 完整覆盖项目默认值，服务声明的 `#[lazy]` 策略仍然优先。
    /// `Some(Default::default())` 明确选择库基线，不等同于继承项目配置的 `None`。
    /// 各次创建共享不可变计划，但运行时、缓存、实例与关闭状态独立；运行时不读取 TOML。
    pub async fn build(options: Option<ServiceProviderOptions>) -> Result<Self, BuildError> {
        // 普通 Cargo 不会生成应用计划。必须明确提示工具链缺失，不能把未经过
        // Nestrs 编译的应用伪装成一个合法的空容器。隔离单元测试仍可装载空计划。
        if !cfg!(any(nestrs_compiler, test)) {
            return Err(BuildError::CompilerPlanUnavailable);
        }
        let application = CompiledApplication::load();
        let options = options.unwrap_or_else(|| application.options.clone());
        let graph = application.graph.clone();
        tokio::runtime::Handle::try_current().map_err(|_| BuildError::RuntimeUnavailable)?;
        let (runtime, owner) = Runtime::start(
            graph.clone(),
            options.max_concurrent_activations.get(),
            options.initialization,
        )
        .await?;
        Ok(Self {
            graph,
            runtime,
            owner,
            scope_initialization: options.scope_initialization,
        })
    }

    /// 获取默认 key 的必选服务。工具链从真实方法调用收集闭合查询类型，查询本身
    /// 仅执行已编译的计划；不存在、生命周期不允许或初始化失败均返回错误。
    pub async fn get_required_service<T: ?Sized + Send + Sync + 'static>(
        &self,
    ) -> Result<&T, ResolveError> {
        self.view().required::<T>(None).await
    }

    /// 获取默认 key 的可选服务。只有未注册返回 None。
    pub async fn get_service<T: ?Sized + Send + Sync + 'static>(
        &self,
    ) -> Result<Option<&T>, ResolveError> {
        self.view().query::<T>(None).await
    }

    /// 使用运行期 key 从已冻结路由中获取服务，不创建新的类型或注册。
    pub async fn get_required_keyed_service<T: ?Sized + Send + Sync + 'static>(
        &self,
        key: ServiceKey,
    ) -> Result<&T, ResolveError> {
        self.view().required::<T>(Some(key)).await
    }

    /// 按精确 key 获取可选服务；初始化失败仍返回错误。
    pub async fn get_keyed_service<T: ?Sized + Send + Sync + 'static>(
        &self,
        key: ServiceKey,
    ) -> Result<Option<&T>, ResolveError> {
        self.view().query::<T>(Some(key)).await
    }

    /// 借用当前 root 的固定计划、运行时和 owner，建立轻量查询视图。
    fn view(&self) -> ServiceProviderRef<'_> {
        ServiceProviderRef {
            graph: &self.graph,
            runtime: &self.runtime,
            owner: &self.owner,
        }
    }

    /// 建立独立 scope，完成选中的 Scoped 初始化后交付。
    ///
    /// `None` 使用容器保存的 scope 默认策略；`Some(options)` 只覆盖本次创建。
    /// `Some(Default::default())` 明确选择 Lazy，不读取容器的 scope 默认值。
    /// 服务级 `#[lazy(false)]` 在 Lazy 下仍生效；所有 scope 共享 root 的构造额度。
    /// 失败先关闭，取消由未交付 owner 发起关闭，不交付部分初始化的 scope。
    pub async fn create_scope(
        &self,
        options: Option<ServiceScopeOptions>,
    ) -> Result<ServiceScope<'_>, ScopeBuildError> {
        let options = options.unwrap_or(ServiceScopeOptions {
            initialization: self.scope_initialization,
        });
        let owner = self.runtime.create_scope(options.initialization).await?;
        Ok(ServiceScope {
            provider: self,
            owner,
        })
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
    /// 借用 root，确保 scope 使用期间其计划和运行时仍有效。
    provider: &'provider ServiceProvider,

    /// 本次操作所属的实际 root 或 scope。
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

    /// 消费当前 scope，等待已接受任务及本 scope 的串行 cleanup 完成。
    /// 查询结果仍被使用时，Rust 借用检查会拒绝消费这个 scope。
    pub async fn dispose_async(self) -> Result<(), DisposeError> {
        self.provider.runtime.close(&self.owner).await
    }
}

/// 提供异步查询方法的轻量视图；临时视图不会缩短结果引用的有效期。
#[derive(Clone, Copy)]
pub struct ServiceProviderRef<'owner> {
    /// 当前入口共享的不可变执行计划。
    graph: &'owner ValidatedGraph,

    /// 用于提交当前 owner 查询与关闭的共享协调器句柄。
    runtime: &'owner Arc<Runtime>,

    /// 本次操作所属的实际 root 或 scope。
    owner: &'owner Arc<Owner>,
}

impl<'owner> ServiceProviderRef<'owner> {
    /// 返回引用绑定实际 owner 的借用期，因此链式 service_provider() 不产生
    /// 临时视图借用错误；查询等待仍可以取消而不取消已接受的初始化。
    pub async fn get_required_service<T: ?Sized + Send + Sync + 'static>(
        self,
    ) -> Result<&'owner T, ResolveError> {
        self.required::<T>(None).await
    }

    /// 默认 key 的可选查询，只有类型未注册返回 None。
    pub async fn get_service<T: ?Sized + Send + Sync + 'static>(
        self,
    ) -> Result<Option<&'owner T>, ResolveError> {
        self.query::<T>(None).await
    }

    /// 精确 key 的必选查询。
    pub async fn get_required_keyed_service<T: ?Sized + Send + Sync + 'static>(
        self,
        key: ServiceKey,
    ) -> Result<&'owner T, ResolveError> {
        self.required::<T>(Some(key)).await
    }

    /// 精确 key 的可选查询。
    pub async fn get_keyed_service<T: ?Sized + Send + Sync + 'static>(
        self,
        key: ServiceKey,
    ) -> Result<Option<&'owner T>, ResolveError> {
        self.query::<T>(Some(key)).await
    }

    /// 将可选路由结果转换为必选查询，缺席时报告准确服务身份。
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

    /// 查找冻结路由并解析真实实例，最终返回受当前 owner 借用约束的引用。
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

        // concrete 查询从实例当前的共享借用恢复准确地址；trait 查询与构造输入、延迟交付
        // 共用直接写入 typed 栈槽的投影。ProjectionTarget 同时核对
        // 结果类型与实例 lease；投影只能创建当前实例的视图，不能更换它的所有者。
        let pointer = if let Some(project) = route.projection {
            let token = ProjectionTarget::project::<T>(InputSlot::new(0), lease, project)
                .map_err(|error| ResolveError::new(error.to_string()))?;
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

// 门面白盒测试仍存放于 tests/。挂载在本模块可直接建立隔离计划，避免为了测试
// 暴露第二个构建入口或替换进程共享的编译计划。
#[cfg(all(test, not(nestrs_compiler_contract)))]
#[path = "../tests/unit/facade_api.rs"]
mod tests;
