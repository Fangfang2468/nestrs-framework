//! static 初始化器与标准库默认方法保留有限闭合查询；各路径使用独立类型。
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
    marker: PhantomData<T>,
}

pub fn constructions() -> usize {
    CONSTRUCTIONS.load(Ordering::SeqCst)
}

pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = Result<usize, ResolveError>> + Send + 'a>>;
pub type QueryFunction = for<'a> fn(&'a ServiceProvider) -> QueryFuture<'a>;

fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) -> QueryFuture<'_> {
    Box::pin(async move {
        provider
            .get_required_service::<Repository<T>>()
            .await
            .map(|repository| repository.id)
    })
}

struct DirectStatic;
struct AssociatedStatic;
struct ConstFnStatic;
struct InlineStatic;
struct UncalledStatic;
struct MutableStatic;
struct ArrayStatic;

struct Holder<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> Holder<T> {
    const QUERY: QueryFunction = query::<T>;
}
const fn select<T: Send + Sync + 'static>() -> QueryFunction {
    query::<T>
}

pub static DIRECT: QueryFunction = query::<DirectStatic>;
pub static ASSOCIATED: QueryFunction = Holder::<AssociatedStatic>::QUERY;
pub static CONST_FN: QueryFunction = select::<ConstFnStatic>();
pub static INLINE: QueryFunction = const { Holder::<InlineStatic>::QUERY };
pub static ARRAY: [QueryFunction; 1] = [query::<ArrayStatic>];
// 与本 crate 的 HIR 规则一致：私有且未调用的闭合静态初始化器也保留查询需求。
#[allow(dead_code)]
static UNCALLED: QueryFunction = query::<UncalledStatic>;
// 可修改的静态函数指针不属于这次跨 crate 固定初始化器摘要，不能增加预热计数。
#[allow(dead_code)]
static mut MUTABLE: QueryFunction = query::<MutableStatic>;

// 不是 provider；查询类型只能由标准库转发调用的真实闭合实参恢复。
pub struct Query<'a, T> {
    provider: &'a ServiceProvider,
    consumed: bool,
    marker: PhantomData<T>,
}
impl<'a, T> Query<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self {
            provider,
            consumed: false,
            marker: PhantomData,
        }
    }
}
impl<'a, T: Send + Sync + 'static> Iterator for Query<'a, T> {
    type Item = QueryFuture<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.consumed {
            None
        } else {
            self.consumed = true;
            Some(query::<T>(self.provider))
        }
    }
}

pub fn generic_iterator<I: Iterator>(iterator: I) -> Vec<I::Item> {
    // 库编译时 I 尚未闭合，不能因为看不到具体 Iterator 就删除这条转发边。
    iterator.collect()
}

pub fn generic_type<T: Send + Sync + 'static>(provider: &ServiceProvider) -> Vec<QueryFuture<'_>> {
    Query::<T>::new(provider).collect()
}

pub struct OuterQuery<'a, T>(Query<'a, T>);
impl<'a, T> OuterQuery<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(Query::new(provider))
    }
}
impl<'a, T: Send + Sync + 'static> Iterator for OuterQuery<'a, T> {
    type Item = QueryFuture<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        // 外层本身没有直接查询，只有另一层标准转发；载体传播必须达到固定点。
        self.0.by_ref().collect::<Vec<_>>().into_iter().next()
    }
}

pub struct OverriddenQuery<'a, T>(Query<'a, T>);
impl<'a, T> OverriddenQuery<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(Query::new(provider))
    }
}
impl<'a, T: Send + Sync + 'static> Iterator for OverriddenQuery<'a, T> {
    type Item = QueryFuture<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }

    fn collect<B: FromIterator<Self::Item>>(self) -> B {
        // 真实 impl 覆盖了标准默认实现；仅看到 collect 名称或 Iterator 类型便
        // 猜测 next 会制造不存在的查询根。这个路径必须保持零个服务实例。
        std::iter::empty().collect()
    }
}

// 泛型 Self 的内部 Vec 不是独立查询载体，不能污染下面完全无关的 Vec<T> 递归。
#[allow(dead_code)]
struct NestedSelf<T>(PhantomData<T>);
#[allow(dead_code)]
trait UnusedQuery {
    fn query(provider: &ServiceProvider) -> QueryFuture<'_>;
}
impl<T: Send + Sync + 'static> UnusedQuery for NestedSelf<Vec<T>> {
    fn query(provider: &ServiceProvider) -> QueryFuture<'_> {
        query::<T>(provider)
    }
}

#[allow(dead_code)]
mod unrelated_static {
    use std::marker::PhantomData;
    type L0 = ();
    type L1 = (L0, L0);
    type L2 = (L1, L1);
    type L3 = (L2, L2);
    type L4 = (L3, L3);
    type L5 = (L4, L4);
    type L6 = (L5, L5);
    type L7 = (L6, L6);
    type L8 = (L7, L7);
    type L9 = (L8, L8);
    type L10 = (L9, L9);
    type L11 = (L10, L10);
    struct Holder<T>(PhantomData<T>);
    fn noop() {}
    impl<T> Holder<T> {
        const CALLBACK: fn() = noop;
    }
    // 类型树很大但原生编译深度很浅；没有查询，不能套用 DI 类型复杂度上限。
    static CALLBACK: fn() = Holder::<L11>::CALLBACK;
}

fn unrelated_recursive<T>() {
    let _empty = Vec::<T>::new();
    if false {
        unrelated_recursive::<Vec<T>>();
    }
}

#[allow(dead_code)]
fn unrelated_closed() {
    if false {
        unrelated_recursive::<u8>();
    }
}
