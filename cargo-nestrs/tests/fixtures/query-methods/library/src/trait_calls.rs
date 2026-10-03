//! 标准 trait 的调用身份来自 core，业务实现及查询摘要来自本 crate。
use nestrs::injectable;
use nestrs_core::{ResolveError, ServiceProvider};
use std::{
    future::Future,
    marker::PhantomData,
    ops::Add,
    pin::Pin,
    sync::atomic::{AtomicUsize, Ordering},
};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
pub struct Repository<T: Send + Sync + 'static> {
    #[value(CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    #[value(PhantomData)]
    marker: PhantomData<T>,
}

pub fn constructions() -> usize {
    CONSTRUCTIONS.load(Ordering::SeqCst)
}

pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = Result<usize, ResolveError>> + Send + 'a>>;

fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    Box::pin(async move {
        provider
            .get_required_service::<Repository<T>>()
            .await
            .map(|repository| repository.id)
    })
}

// 刻意不是 provider：T 必须由真实调用点恢复，不能依赖已知 provider 的方法补根。
pub struct Query<'a, T>(&'a ServiceProvider, PhantomData<T>);

impl<'a, T: Send + Sync + 'static> Query<'a, T> {
    pub const LABEL: &'static str = "query";

    pub fn new(provider: &'a ServiceProvider) -> Self {
        // 关联常量也有 type_dependent_def_id，扩展调用收集时不能错当成函数项。
        assert_eq!(Self::LABEL, "query");
        Self(provider, PhantomData)
    }

    pub fn inherent_query(self) -> QueryFuture<'a> {
        query::<T>(self.0)
    }
}

impl<'a, T: Send + Sync + 'static> Iterator for Query<'a, T> {
    type Item = QueryFuture<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(query::<T>(self.0))
    }
}

impl<'a, T: Send + Sync + 'static> Add<()> for Query<'a, T> {
    type Output = QueryFuture<'a>;

    fn add(self, (): ()) -> Self::Output {
        query::<T>(self.0)
    }
}

pub trait CustomQuery<'a> {
    fn custom_query(self) -> QueryFuture<'a>;
}

impl<'a, T: Send + Sync + 'static> CustomQuery<'a> for Query<'a, T> {
    fn custom_query(self) -> QueryFuture<'a> {
        query::<T>(self.0)
    }
}

pub fn iterator_query<I: Iterator>(mut iterator: I) -> I::Item {
    iterator.next().expect("query iterator always has one item")
}

struct OnlyUpstreamDeadBranch;

// 私有函数中未执行的闭合 trait 调用也要跨 crate 保留；Release 优化不能删根。
fn uncalled(provider: &ServiceProvider) {
    if false {
        drop(Query::<OnlyUpstreamDeadBranch>::new(provider).next());
    }
}
