//! 实例的迭代释放队列，与 Tokio 和服务调度器无关。
//!
//! 直接让 `Arc` 沿依赖关系连续析构，会把深依赖链变成同样深的调用栈。本模块让最后
//! 一个 lease 只提交载荷；一个排空者用循环销毁队列。析构时释放出的依赖重新入队，
//! 不在当前调用栈继续向下析构。调用用户 Drop 时绝不持有队列锁。
//!
//! 每个主动释放操作还有一个完成组：同线程重入产生的依赖析构归入同一组，其他线程
//! 同时提交的无关实例归入各自的组。因此逻辑关闭可以等待自己的释放结果，并汇总
//! 这次释放中的析构 panic，而不会误报其他线程的失败。

use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    panic::{AssertUnwindSafe, catch_unwind},
    pin::Pin,
    rc::Rc,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use super::instance::InstancePayload;
use crate::panic_payload::PanicPayload;

/// 同一 root 及其 scopes 共用的同步释放域；不依赖异步执行器。
#[derive(Default)]
pub(crate) struct ReleaseDomain {
    state: Mutex<ReleaseState>,
}

#[derive(Default)]
struct ReleaseState {
    pending: VecDeque<PendingRelease>,
    // 同一域同时只有一个排空者；其他线程和重入析构只追加队列。
    draining: bool,
}

impl ReleaseDomain {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub(super) fn release(&self, payload: InstancePayload) {
        let active = ACTIVE_RELEASE
            .try_with(|active| active.borrow().clone())
            .ok()
            .flatten();
        let nested = active.is_some();
        // 重入的依赖释放属于发起它的完成组；其他线程排队的释放有各自的结果。
        // 仅当前调用取得排空权且本线程未展开栈时，才在排空后恢复自己的首个 panic。
        // 不把其他线程组的 panic 抛给当前线程，也不在重入的释放中展开栈。
        let propagate = !nested && !std::thread::panicking();
        if let Some(panic) = self.enqueue(payload, active.unwrap_or_default(), propagate) {
            panic.resume();
        }
    }

    pub(super) fn release_tracked(&self, payload: InstancePayload) -> ReleaseCompletion {
        let group = Arc::new(ReleaseGroup::default());
        self.enqueue(
            payload,
            ActiveRelease {
                group: group.clone(),
                propagation: None,
            },
            false,
        );
        ReleaseCompletion(group)
    }

    fn enqueue(
        &self,
        payload: InstancePayload,
        mut origin: ActiveRelease,
        propagate: bool,
    ) -> Option<PanicPayload> {
        origin.group.begin();
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.pending.push_back(PendingRelease {
                payload,
                group: origin.group.clone(),
            });
            if state.draining {
                return None;
            }
            state.draining = true;
        }
        // 原始 panic 只在同步调用链与 TLS 间暂存，从不进入队列或共享完成组。
        // 跨域的同线程重入可写入此槽，但只有最外层普通 Drop 有权恢复它。
        if propagate {
            origin.propagation = Some(Rc::default());
        }
        loop {
            let pending = {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                match state.pending.pop_front() {
                    Some(pending) => pending,
                    None => {
                        state.draining = false;
                        break;
                    }
                }
            };
            let PendingRelease { payload, group } = pending;
            let service = payload.service_name();
            // 先离开锁作用域，再执行用户析构。析构期间释放的依赖加入当前完成组，
            // 入队后由外层循环处理，既不会死锁，也不会形成递归析构调用栈。
            let propagation = if Arc::ptr_eq(&origin.group, &group) {
                origin.propagation.clone()
            } else {
                None
            };
            let context = ReleaseContext::enter(ActiveRelease {
                group: group.clone(),
                propagation: propagation.clone(),
            });
            let failure = catch_unwind(AssertUnwindSafe(|| drop(payload)))
                .err()
                .and_then(|payload| {
                    let panic = PanicPayload::new(payload);
                    if let Some(slot) = &propagation {
                        let mut first = slot.borrow_mut();
                        if first.is_none() {
                            *first = Some(panic);
                            return None;
                        }
                    }
                    // 不持有槽位借用或队列锁执行用户 Drop。panic 载荷也可能持有
                    // lease；它的依赖须先 begin，再 finish 父项，避免过早完成或归错组。
                    Some(format!("{service}: {}", panic.into_message()))
                });
            drop(context);
            group.finish(failure);
        }
        if propagate {
            origin.propagation.and_then(|slot| slot.borrow_mut().take())
        } else {
            None
        }
    }
}

struct PendingRelease {
    payload: InstancePayload,
    group: Arc<ReleaseGroup>,
}

thread_local! {
    // 只传播当前同步析构的完成组，不保存实例；TLS 销毁期间不可用时允许直接跳过。
    static ACTIVE_RELEASE: RefCell<Option<ActiveRelease>> = const { RefCell::new(None) };
}

#[derive(Clone, Default)]
struct ActiveRelease {
    group: Arc<ReleaseGroup>,
    // 只在同线程同步重入中共享，不能随 PendingRelease 进入另一个线程的排空者。
    propagation: Option<Rc<RefCell<Option<PanicPayload>>>>,
}

/// 恢复之前的线程局部上下文，使重入、析构 panic 和 TLS 销毁都不会遗留错误状态。
struct ReleaseContext(Option<ActiveRelease>);
impl ReleaseContext {
    fn enter(context: ActiveRelease) -> Self {
        Self(
            ACTIVE_RELEASE
                .try_with(|active| active.replace(Some(context)))
                .ok()
                .flatten(),
        )
    }
}
impl Drop for ReleaseContext {
    fn drop(&mut self) {
        let _ = ACTIVE_RELEASE.try_with(|active| active.replace(self.0.take()));
    }
}

#[derive(Default)]
struct ReleaseProgress {
    // 包含尚未开始和正在析构的载荷；重入依赖必须先 begin，再让父载荷 finish。
    pending: usize,
    // 完成组只存文本；poll、丢弃回执或最后一个 Arc 均不再执行用户载荷的 Drop。
    failures: Vec<String>,
    waker: Option<Waker>,
}

#[derive(Default)]
struct ReleaseGroup(Mutex<ReleaseProgress>);
impl ReleaseGroup {
    fn begin(&self) {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .pending += 1;
    }

    fn finish(&self, failure: Option<String>) {
        let waker = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.failures.extend(failure);
            state.pending -= 1;
            if state.pending == 0 {
                state.waker.take()
            } else {
                None
            }
        };
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

/// 当前载荷及其重入依赖释放的完成回执，不依赖特定异步执行器。
///
/// 丢弃回执只取消等待，不取消已经开始的析构。回执不持有实例 lease，因此自身不会
/// 阻止等待中的实例释放；失败信息在整个完成组排空后一次性交付。
pub(crate) struct ReleaseCompletion(Arc<ReleaseGroup>);
impl Future for ReleaseCompletion {
    type Output = Vec<String>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let failures = {
            let mut state = self.0.0.lock().unwrap_or_else(|error| error.into_inner());
            if state.pending != 0 {
                state.waker = Some(context.waker().clone());
                return Poll::Pending;
            }
            std::mem::take(&mut state.failures)
        };
        Poll::Ready(failures)
    }
}
