//! 面向业务字段与 factory 参数的只读延迟注入句柄。
//!
//! 与普通 `Injection` 一样，此类型只承担定位、保活和访问入口。延迟状态集中在一个
//! 地址稳定的内部槽位中，句柄移动不会移动槽位；业务借用结束前，Box 不可能被释放。
//! Box 已编码非空指针及独占所有权，无须再手写裸指针、析构器或第二份槽位 lease。

use super::{InputSlot, LazyDependency, deferred::DeferredSlot};
use crate::{ResolveError, service::Injectable};

/// `#[inject] #[lazy]` 字段或 `#[lazy]` factory 参数使用的只读延迟注入句柄。
///
/// [`Self::get`] 首次取得目标时可能等待异步构造，成功后重复访问返回同一个实例。
/// Singleton / Scoped 仍复用生命周期缓存；Transient 则以当前字段或参数为独立消费槽位。
/// 初始化错误也保留在当前句柄中，调用不会隐式重试。
/// factory 参数按值交付此句柄，可跨 await 并移入返回的服务；普通参数仍是帧内借用。
///
/// 此类型不提供 `Deref`、`Clone`、可变访问或公开构造函数。返回引用受当前句柄借用
/// 限制，内部强 lease 保证实例地址有效；owner cleanup 后不保证业务资源仍可使用。
pub struct LazyInjection<T: ?Sized> {
    slot: Box<DeferredSlot<T>>,
}

impl<T: ?Sized> LazyInjection<T>
where
    T: Injectable,
{
    pub(crate) fn new(dependency: LazyDependency, slot: InputSlot) -> Self {
        Self {
            slot: Box::new(DeferredSlot::new(dependency, slot)),
        }
    }

    /// 异步取得句柄所指向的服务，并在当前句柄存活期间保留它的强 lease。
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
        self.slot.get().await
    }
}

#[cfg(test)]
#[path = "../../tests/unit/activation/lazy.rs"]
mod tests;
