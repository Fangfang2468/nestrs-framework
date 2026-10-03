//! 上游泛型关联常量的初始化器、函数项及未执行分支都必须保留为跨 crate 查询摘要。
use nestrs::injectable;
use nestrs_core::{ResolveError, ServiceProvider};
use std::{
    future::Future,
    marker::PhantomData,
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
pub type QueryFunction = for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a>;

// 唯一真正调用 core 查询的地方保持泛型，具体类型只能从关联常量的使用处恢复。
fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    Box::pin(async move {
        provider
            .get_required_service::<Repository<T>>()
            .await
            .map(|repository| repository.id)
    })
}

pub struct Holder<T>(PhantomData<T>);

impl<T: Send + Sync + 'static> Holder<T> {
    pub const QUERY: QueryFunction = query::<T>;
    pub const CHAIN: QueryFunction = Self::QUERY;
    pub const THROUGH_CONST_FN: QueryFunction = select::<T>();
}

const fn select<T: Send + Sync + 'static>() -> QueryFunction {
    query::<T>
}

pub fn inline_query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    // 上游摘要须保留函数到内联 const 的引用，供下游传入 T 后继续恢复关联常量函数项。
    let query = const { Holder::<T>::QUERY };
    query(provider)
}

pub trait DefaultQuery<T: Send + Sync + 'static> {
    const QUERY: QueryFunction = query::<T>;
}

impl<T: Send + Sync + 'static> DefaultQuery<T> for Holder<T> {}

pub trait ExplicitQuery<T: Send + Sync + 'static> {
    const QUERY: QueryFunction;
}

impl<T: Send + Sync + 'static> ExplicitQuery<T> for Holder<T> {
    const QUERY: QueryFunction = query::<T>;
}

pub fn generic_trait_query<H, T>(provider: &ServiceProvider) -> QueryFuture<'_>
where
    H: ExplicitQuery<T>,
    T: Send + Sync + 'static,
{
    // producer 编译时 H 未闭合，必须保留 trait 身份与泛型实参供下游选择实际 impl。
    <H as ExplicitQuery<T>>::QUERY(provider)
}

struct DefaultBranch<T>(PhantomData<T>);
struct OverriddenBranch<T>(PhantomData<T>);

pub trait OverriddenQuery<T: Send + Sync + 'static> {
    const QUERY: QueryFunction = query::<DefaultBranch<T>>;
}

impl<T: Send + Sync + 'static> OverriddenQuery<T> for Holder<T> {
    // 精确计数既要发现漏掉 override，也要拒绝把不执行的默认分支额外编进计划。
    const QUERY: QueryFunction = query::<OverriddenBranch<T>>;
}

struct OnlyUpstreamDeadBranch;

// 没有应用调用这个函数。它的闭合常量引用仍须保留，不能被 Release 优化删掉。
fn uncalled(provider: &ServiceProvider) {
    if false {
        drop(Holder::<OnlyUpstreamDeadBranch>::QUERY(provider));
    }
}

#[cfg(feature = "extra-root")]
struct OnlyFeatureDeadBranch;

#[cfg(feature = "extra-root")]
fn feature_uncalled(provider: &ServiceProvider) {
    if false {
        drop(Holder::<OnlyFeatureDeadBranch>::CHAIN(provider));
    }
}
