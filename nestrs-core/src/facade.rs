//! `nestrs-core` 的公开 DI 门面。
//!
//! 此模块目前只固定 API 契约，不包含注册收集、图编译、实例存储或 Tokio 调度实现。
//! 调用任一方法都会触发明确的 `unimplemented!()` 占位；后续实现必须遵守这里已经
//! 固定的所有权与生命周期形状，而不能把内部 store、裸指针或 activation plan 暴露出去。

use std::marker::PhantomData;

use crate::ServiceKey;
use thiserror::Error;

/// 构建服务容器时发生的错误。
///
/// 错误的具体分类将在图编译实现落地时补充；公开类型现在先固定，避免把内部 graph
/// 或实例存储模型固化为用户 API。
#[derive(Debug, Error)]
#[error("服务容器构建失败")]
pub struct BuildError {
    _private: (),
}

/// 获取或激活服务时发生的错误。
#[derive(Debug, Error)]
#[error("服务解析失败")]
pub struct ResolveError {
    _private: (),
}

/// 关闭 provider 或 scope 时发生的错误。
#[derive(Debug, Error)]
#[error("服务容器关闭失败")]
pub struct ShutdownError {
    _private: (),
}

/// 应用级 DI 容器。
///
/// `ServiceProvider` 是 Singleton 的 owner，并可创建 [`ServiceScope`]。它不提供运行期
/// 注册、替换注册或按 `TypeId` 查询的能力；服务只能通过静态类型参数获取。
#[must_use]
pub struct ServiceProvider {
    _private: (),
}

impl ServiceProvider {
    /// 收集服务声明并建立 root provider。
    ///
    /// 后续实现将在当前 Tokio runtime 中完成静态图验证和必要的异步初始化。
    pub async fn build() -> Result<Self, BuildError> {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }

    /// 从 root provider 获取默认 key 的服务。
    ///
    /// Singleton 由 provider 持有；root provider 请求 Scoped 服务将返回解析错误。
    /// 直接解析的 Transient 由 provider 的生命周期 journal 持有，直至 provider 关闭。
    pub async fn get<T>(&self) -> Result<&T, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }

    /// 从 root provider 尝试获取默认 key 的服务。
    pub async fn try_get<T>(&self) -> Result<Option<&T>, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }

    /// 从 root provider 获取指定 key 的服务。
    pub async fn get_keyed<T>(&self, _key: ServiceKey) -> Result<&T, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }

    /// 从 root provider 尝试获取指定 key 的服务。
    pub async fn try_get_keyed<T>(&self, _key: ServiceKey) -> Result<Option<&T>, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }

    /// 创建一个新的 Scoped 生命周期边界。
    ///
    /// 创建 scope 本身不解析任何服务，因此它保持同步；第一次 `get` 才会按需激活服务。
    pub fn create_scope(&self) -> ServiceScope<'_> {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }

    /// 执行 provider 及其已创建服务的异步关闭流程。
    pub async fn shutdown(self) -> Result<(), ShutdownError> {
        unimplemented!("ServiceProvider runtime has not been implemented")
    }
}

/// 一次 Scoped 生命周期的 owner。
///
/// scope 借用其 [`ServiceProvider`]；因此在 scope 仍然存在时，安全 Rust 不能消费
/// provider 并调用 `shutdown`。它不暴露 cache、activation plan 或内部实例地址。
#[must_use = "ServiceScope 必须在结束前调用 shutdown().await"]
pub struct ServiceScope<'provider> {
    _provider: PhantomData<&'provider ServiceProvider>,
}

impl ServiceScope<'_> {
    /// 从当前 scope 获取默认 key 的服务。
    ///
    /// Singleton 从 root provider 读取，Scoped 缓存在当前 scope；每次获取 Transient
    /// 都创建新的实例，并由本 scope 持有到关闭为止。
    pub async fn get<T>(&self) -> Result<&T, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceScope runtime has not been implemented")
    }

    /// 从当前 scope 尝试获取默认 key 的服务。
    pub async fn try_get<T>(&self) -> Result<Option<&T>, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceScope runtime has not been implemented")
    }

    /// 从当前 scope 获取指定 key 的服务。
    pub async fn get_keyed<T>(&self, _key: ServiceKey) -> Result<&T, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceScope runtime has not been implemented")
    }

    /// 从当前 scope 尝试获取指定 key 的服务。
    pub async fn try_get_keyed<T>(&self, _key: ServiceKey) -> Result<Option<&T>, ResolveError>
    where
        T: ?Sized + Send + Sync + 'static,
    {
        unimplemented!("ServiceScope runtime has not been implemented")
    }

    /// 执行 scope 及其已创建服务的异步关闭流程。
    pub async fn shutdown(self) -> Result<(), ShutdownError> {
        unimplemented!("ServiceScope runtime has not been implemented")
    }
}
