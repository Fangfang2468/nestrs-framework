//! 本地泛型关联常量必须把保存的函数项和闭合实参继续传给查询根收集器。
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
struct Repository<T: Send + Sync + 'static> {
    #[value(CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    #[value(PhantomData)]
    marker: PhantomData<T>,
}

type QueryFuture<'a> = Pin<Box<dyn Future<Output = Result<usize, ResolveError>> + Send + 'a>>;
type QueryFunction = for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a>;

fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    Box::pin(async move {
        provider
            .get_required_service::<Repository<T>>()
            .await
            .map(|repository| repository.id)
    })
}

// Holder 刻意不是 provider，不能由已知 provider 的方法扫描意外补出 T。
struct Holder<T>(PhantomData<T>);

impl<T: Send + Sync + 'static> Holder<T> {
    const QUERY: QueryFunction = query::<T>;
    const CHAIN: QueryFunction = Self::QUERY;
    const THROUGH_CONST_FN: QueryFunction = select::<T>();
}

// 收集的是 const fn 体中的函数项摘要；不靠运行 query 或构造服务解释这个常量。
const fn select<T: Send + Sync + 'static>() -> QueryFunction {
    query::<T>
}

fn inline_query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    // 内联 const 有独立 HIR body，必须从外层泛型函数继续传入闭合 T，不能丢失这条边。
    let query = const { Holder::<T>::QUERY };
    query(provider)
}

trait DefaultQuery<T: Send + Sync + 'static> {
    const QUERY: QueryFunction = query::<T>;
}

impl<T: Send + Sync + 'static> DefaultQuery<T> for Holder<T> {}

trait ExplicitQuery<T: Send + Sync + 'static> {
    const QUERY: QueryFunction;
}

impl<T: Send + Sync + 'static> ExplicitQuery<T> for Holder<T> {
    const QUERY: QueryFunction = query::<T>;
}

fn generic_trait_query<H, T>(provider: &ServiceProvider) -> QueryFuture<'_>
where
    H: ExplicitQuery<T>,
    T: Send + Sync + 'static,
{
    // 记录本函数时 H 仍然开放，只有调用点提供真实实现和关联常量的定义身份。
    <H as ExplicitQuery<T>>::QUERY(provider)
}

struct DefaultBranch<T>(PhantomData<T>);
struct OverriddenBranch<T>(PhantomData<T>);

trait OverriddenQuery<T: Send + Sync + 'static> {
    const QUERY: QueryFunction = query::<DefaultBranch<T>>;
}

impl<T: Send + Sync + 'static> OverriddenQuery<T> for Holder<T> {
    // 本次闭合类型只能贡献实际选择的分支；不能额外物化被覆盖的默认常量。
    const QUERY: QueryFunction = query::<OverriddenBranch<T>>;
}

struct ThroughInherent;
struct ThroughChain;
struct ThroughDefault;
struct ThroughExplicit;
struct ThroughGenericTrait;
struct ThroughOverride;
struct ThroughConstFn;
struct ThroughInlineConst;
struct OnlyDeadBranch;
#[cfg(feature = "extra-root")]
struct ThroughFeature;

pub fn constructions() -> usize {
    CONSTRUCTIONS.load(Ordering::SeqCst)
}

pub fn expected_constructions() -> usize {
    9 + usize::from(cfg!(feature = "extra-root"))
}

pub async fn verify(provider: &ServiceProvider) {
    // 八条真实路径和一个未执行分支使用不同 T；任何其他路径都不能替它们补根。
    // 先取出函数指针再调用，确保测试覆盖常量初始化器，而非直接闭合函数项调用。
    let query = Holder::<ThroughInherent>::QUERY;
    let mut ids = vec![
        query(provider).await.unwrap(),
        Holder::<ThroughChain>::CHAIN(provider).await.unwrap(),
        <Holder<ThroughDefault> as DefaultQuery<ThroughDefault>>::QUERY(provider)
            .await
            .unwrap(),
        <Holder<ThroughExplicit> as ExplicitQuery<ThroughExplicit>>::QUERY(provider)
            .await
            .unwrap(),
        generic_trait_query::<Holder<ThroughGenericTrait>, ThroughGenericTrait>(provider)
            .await
            .unwrap(),
        <Holder<ThroughOverride> as OverriddenQuery<ThroughOverride>>::QUERY(provider)
            .await
            .unwrap(),
        Holder::<ThroughConstFn>::THROUGH_CONST_FN(provider)
            .await
            .unwrap(),
        inline_query::<ThroughInlineConst>(provider).await.unwrap(),
    ];
    if false {
        drop(Holder::<OnlyDeadBranch>::QUERY(provider));
    }
    #[cfg(feature = "extra-root")]
    ids.push(Holder::<ThroughFeature>::QUERY(provider).await.unwrap());
    ids.sort_unstable();
    assert!(ids.windows(2).all(|pair| pair[0] != pair[1]));
    assert_eq!(constructions(), expected_constructions());
}
