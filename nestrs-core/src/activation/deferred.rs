//! 单个延迟消费槽位的稳定存储与交付状态。
//!
//! 一个槽位同时保存“已提交的请求”和“已交付的类型化结果”：前者让取消后的访问
//! 接续同一 occurrence，后者持有真实 lease 并提供稳定借用。两者共用一个 Box 分配，
//! 不再为每个字段另建运行时 resolver。运行时只通过下面的协议接收首次请求。
//! 固定依赖描述由计划共享；这里仅保存描述的引用及当前字段独有的动态状态。

use std::sync::Mutex;

use tokio::sync::{OnceCell, watch};

use super::{DependencyLease, Injection, InputSlot, LazyDependency};
use crate::{ResolveError, service::Injectable};

/// 接收端属于槽位，发送端属于协调器；等待 future 不拥有初始化任务。
pub(crate) type LazyReceiver = watch::Receiver<Option<Result<DependencyLease, ResolveError>>>;

/// owner 提供的首次请求入口，activation 不依赖具体 runtime 类型。
///
/// 调用必须同步完成提交并返回接收端，不能在二者之间挂起。槽位只弱引用此对象，
/// 避免 owner journal → 消费者 → 槽位 → owner 的强引用环。
pub(crate) trait LazyResolver: Send + Sync {
    fn request(&self, provider: usize) -> Result<LazyReceiver, &'static str>;
}

/// `LazyInjection<T>` 独占此槽位，借用句柄就同时保活了槽位与已交付的目标实例。
///
/// 不在结构体声明上要求 `T: Injectable`，保留相互引用的 Rust 服务类型进行
/// Send/Sync 共归纳检查的能力；实际交付方法再约束可注入类型。
pub(super) struct DeferredSlot<T: ?Sized> {
    dependency: LazyDependency,
    // OnceCell 的初始化 future 可以被取消，因此 receiver 不能只放在该 future 中。
    // 锁仅覆盖同步读取/保存/取走，不跨 await，也不在持锁时释放用户实例。
    receiver: Mutex<Option<LazyReceiver>>,
    resolved: OnceCell<Result<Injection<T>, ResolveError>>,
}

impl<T: Injectable + ?Sized> DeferredSlot<T> {
    pub(super) fn new(dependency: LazyDependency, input: InputSlot) -> Self {
        // 输入位置属于共享计划。生成的构造适配器只能把描述交付给同一个槽位。
        assert_eq!(
            dependency.plan.input, input,
            "延迟输入与编译计划的输入槽位不一致"
        );
        Self {
            dependency,
            receiver: Mutex::new(None),
            resolved: OnceCell::new(),
        }
    }

    pub(super) async fn get(&self) -> Result<&T, ResolveError> {
        if let Some(result) = self.resolved.get() {
            return result.as_ref().map(|token| &**token).map_err(Clone::clone);
        }
        // 每个尚未交付的调用者都先检查，再争取 OnceCell 的初始化许可。若只在
        // initializer 中检查，另一个构造 worker 会先排队等待，无法及时拒绝死锁。
        // 回调不依赖 owner 存活；已接受请求在 owner 关闭后仍可从 receiver 交付结果。
        self.dependency.check_wait_allowed()?;
        let result = self.resolved.get_or_init(|| self.initialize()).await;

        // 必须先发布类型化结果，再移除请求接收端。此处到返回前没有 await，取消不能
        // 留下“丢失已接受请求、又未保存结果”的状态。只保留最终 token 所需的 lease。
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(receiver);
        result.as_ref().map(|token| &**token).map_err(Clone::clone)
    }

    /// 同步建立唯一请求，并在第一次可能挂起之前将接收端写入槽位。
    fn subscribe(&self) -> Result<LazyReceiver, ResolveError> {
        let existing = self
            .receiver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(receiver) = existing {
            return Ok(receiver);
        }
        // initialize 独占 OnceCell 的初始化许可，request 到保存之间又没有 await，
        // 因此无需持 receiver 锁调用协议。request 结束会释放临时 owner 强引用；若
        // 它恰是最后一份，可能销毁 journal 中的用户实例，必须让此过程发生在锁外。
        let request = self.dependency.request()?;
        let previous = self
            .receiver
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .replace(request.clone());
        debug_assert!(previous.is_none(), "单个初始化者只能提交一次请求");
        drop(previous);
        Ok(request)
    }

    async fn initialize(&self) -> Result<Injection<T>, ResolveError> {
        let mut receiver = self.subscribe()?;
        let lease = loop {
            // 克隆结果后立即释放 watch 读锁，不能带着读锁等待协调器发布。
            let result = receiver.borrow().clone();
            if let Some(result) = result {
                break result.map_err(|error| self.dependency.dependency_error(error))?;
            }
            receiver.changed().await.map_err(|_| {
                self.dependency
                    .error("Tokio 协调器已停止，延迟初始化未完成".into())
            })?;
        };
        // 后续投影与 token 交付全程同步。地址由实际 projector 检查，成功 token 自带
        // 强 lease；这里既不伪造借用，也不改变目标实例经过 ReleaseDomain 释放的路径。
        self.dependency.prepare(lease)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/activation/deferred.rs"]
mod tests;
