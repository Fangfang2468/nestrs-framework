//! 延迟字段的类型化句柄与运行时之间的最小协议。
//!
//! 图编译器已经选定 provider 和真实投影，本模块不再次选择服务。运行时交付一个
//! `LazyResolver`，负责把同一字段的请求合并成一次激活；这里仅把返回 lease 转换为
//! 准确的 `Injection<T>`。两层缓存职责不同：运行时缓存已接受的 occurrence，使取消
//! 等待不会重复构造 Transient；本地缓存类型化令牌，使返回的借用绑定当前字段。

use std::{future::Future, pin::Pin, sync::Arc};

use tokio::sync::OnceCell;

use super::{DependencyLease, Injection, InputPreparer, InputSlot};
use crate::{ResolveError, service::Injectable};

tokio::task_local! {
    /// worker 构造期间禁止同步等待一个尚未完成的延迟字段。
    ///
    /// 构造任务占据全局激活名额；若它再等待需要名额的目标，低并发配置会死锁。
    /// 该标记由 worker 围绕完整构造 future 设置，不能使用跨线程不可靠的线程局部量。
    pub(crate) static IN_ACTIVATION: ();
}

/// 运行时负责 occurrence 身份、关闭检查和等待者取消；activation 只依赖此协议。
///
/// 实现不能强持有消费者所属 owner，否则会形成 journal → 消费者 → owner 的环。
pub(crate) trait LazyResolver: Send + Sync {
    fn resolve(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<DependencyLease, ResolveError>> + Send + '_>>;

    /// 给本地类型化交付错误补齐 provider、来源和依赖路径。
    fn error(&self, message: String) -> ResolveError;
}

/// 冻结计划交给输入准备器的延迟依赖；没有地址，也不表示实例已经存在。
///
/// 类型在私有模块中公开仅为生成 adapter 的函数签名服务，业务没有构造入口。
#[doc(hidden)]
#[derive(Clone)]
pub struct LazyDependency {
    pub(crate) resolver: Arc<dyn LazyResolver>,
    pub(crate) preparer: InputPreparer,
    pub(crate) optional: bool,
}

/// `#[inject] #[lazy]` 字段使用的只读延迟注入句柄。
///
/// [`Self::get`] 首次取得目标时可能等待异步构造，成功后重复访问返回同一个实例。
/// Singleton / Scoped 仍复用生命周期缓存；Transient 则以当前字段为独立消费槽位。
/// 初始化错误也保留在当前句柄中，调用不会隐式重试。
///
/// 此类型不提供 `Deref`、`Clone`、可变访问或公开构造函数。返回引用受当前句柄借用
/// 限制，内部强 lease 保证实例地址有效；owner cleanup 后不保证业务资源仍可使用。
pub struct LazyInjection<T: ?Sized> {
    dependency: LazyDependency,
    slot: InputSlot,
    resolved: OnceCell<Result<Injection<T>, ResolveError>>,
}

impl<T: ?Sized> LazyInjection<T>
where
    T: Injectable,
{
    pub(crate) fn new(dependency: LazyDependency, slot: InputSlot) -> Self {
        Self {
            dependency,
            slot,
            resolved: OnceCell::new(),
        }
    }

    /// 异步取得字段所指向的服务，并在当前句柄存活期间保留它的强 lease。
    ///
    /// 多个调用共享初始化结果。取消调用只取消等待；运行时已接受的初始化继续，
    /// 后续调用仍连接同一个 occurrence。目标构造失败返回带来源路径的解析错误。
    ///
    /// 在当前构造 worker 上下文中首次访问尚未完成的延迟字段会返回错误，以免该
    /// worker 占据激活名额等待另一个 worker。此拒绝不会缓存，服务发布后可正常访问。
    ///
    /// 此保护不会继承到用户自行 `tokio::spawn` / `spawn_blocking` 的任务。构造函数
    /// 或 factory 也不得通过自行创建的任务间接等待未完成的延迟依赖；框架不能自动
    /// 识别任意用户任务之间的等待关系，构造阶段需要的依赖应声明为普通注入。
    pub async fn get(&self) -> Result<&T, ResolveError> {
        if let Some(result) = self.resolved.get() {
            return result.as_ref().map(|token| &**token).map_err(Clone::clone);
        }
        if IN_ACTIVATION.try_with(|()| ()).is_ok() {
            return Err(self.dependency.resolver.error(
                "服务构造期间不能首次获取尚未完成的延迟注入；请声明普通注入依赖，或在服务发布后调用 LazyInjection::get".to_owned(),
            ));
        }

        let result = self
            .resolved
            .get_or_init(|| async {
                let lease = self.dependency.resolver.resolve().await?;
                let prepared = (self.dependency.preparer)(self.slot, Some(lease.erased_ref()))
                    .map_err(|error| self.dependency.resolver.error(error.to_string()))?;
                // 复用冻结图选择的 preparer：trait 的 fat pointer 必须由真实类型化投影
                // 产生，不能用名字比较或在这里再实现一套 concrete-to-trait 转换。
                if self.dependency.optional {
                    prepared
                        .into_optional::<T>(self.slot)
                        .map_err(|error| self.dependency.resolver.error(error.to_string()))?
                        .ok_or_else(|| {
                            self.dependency
                                .resolver
                                .error("已经选中 provider 的延迟可选依赖未交付实例".to_owned())
                        })
                } else {
                    prepared
                        .into_required::<T>(self.slot)
                        .map_err(|error| self.dependency.resolver.error(error.to_string()))
                }
            })
            .await;
        result.as_ref().map(|token| &**token).map_err(Clone::clone)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/activation/lazy.rs"]
mod tests;
