//! 上游库只贡献声明和初始化策略，不根据自身配置提前选取下游启动根。
use nestrs::{factory, injectable, lazy};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static DEFERRED: AtomicUsize = AtomicUsize::new(0);
static FORCED: AtomicUsize = AtomicUsize::new(0);
static GENERIC: AtomicUsize = AtomicUsize::new(0);

#[lazy]
#[injectable]
pub struct DeferredClass {
    #[value(DEFERRED.fetch_add(1, Ordering::SeqCst))]
    pub id: usize,
}

pub trait Port: Send + Sync {
    fn id(&self) -> usize;
}
struct DeferredFactory(usize);
impl Port for DeferredFactory {
    fn id(&self) -> usize {
        self.0
    }
}
#[factory]
#[lazy(true)]
async fn deferred_factory() -> DeferredFactory {
    DeferredFactory(DEFERRED.fetch_add(1, Ordering::SeqCst))
}

pub struct ForcedClient;
#[lazy(false)]
#[factory]
fn forced_client() -> ForcedClient {
    FORCED.fetch_add(1, Ordering::SeqCst);
    ForcedClient
}

#[injectable]
#[lazy(false)]
pub struct Cache<T> {
    marker: PhantomData<T>,
    #[value(GENERIC.fetch_add(1, Ordering::SeqCst))]
    pub id: usize,
}

pub fn counts() -> (usize, usize, usize) {
    (
        DEFERRED.load(Ordering::SeqCst),
        FORCED.load(Ordering::SeqCst),
        GENERIC.load(Ordering::SeqCst),
    )
}
pub fn reset() {
    LAZY_PARAMETERS.store(0, Ordering::SeqCst);
    DEFERRED.store(0, Ordering::SeqCst);
    FORCED.store(0, Ordering::SeqCst);
    GENERIC.store(0, Ordering::SeqCst);
}

// 下游只看到公开 service/trait，私有 factory 的延迟参数仍必须进入最终冻结图。
static LAZY_PARAMETERS: AtomicUsize = AtomicUsize::new(0);
struct ParameterTarget;
impl Port for ParameterTarget {
    fn id(&self) -> usize {
        91
    }
}
#[factory(key = "parameter")]
#[lazy]
fn parameter_target() -> ParameterTarget {
    LAZY_PARAMETERS.fetch_add(1, Ordering::SeqCst);
    ParameterTarget
}

pub struct DeferredConsumer {
    target: nestrs_core::LazyInjection<dyn Port>,
}
impl DeferredConsumer {
    pub async fn target_id(&self) -> usize {
        self.target.get().await.unwrap().id()
    }
}
#[factory]
#[lazy]
fn deferred_consumer(
    #[inject("parameter")]
    #[lazy]
    target: dyn Port,
) -> DeferredConsumer {
    DeferredConsumer { target }
}
pub fn parameter_count() -> usize {
    LAZY_PARAMETERS.load(Ordering::SeqCst)
}
