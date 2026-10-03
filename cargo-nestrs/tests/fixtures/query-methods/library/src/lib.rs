//! 查询语义摘要必须跨 crate 保留，不能受函数是否可达或 LLVM 优化影响。
#![allow(dead_code)]
use nestrs::injectable;
use nestrs_core::ResolveError;
pub use nestrs_core::ServiceProvider;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};

pub mod trait_calls;

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
pub struct Repository<T: Send + Sync + 'static> {
    #[value(CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    #[value(PhantomData)]
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Repository<T> {
    pub async fn get_self(provider: &ServiceProvider) -> Result<&Self, ResolveError> {
        provider.get_required_service::<Self>().await
    }
    pub fn id(&self) -> usize {
        self.id
    }
}

pub async fn generic_query<T: Send + Sync + 'static>(
    provider: &ServiceProvider,
) -> Result<&T, ResolveError> {
    provider.get_required_service::<T>().await
}

pub async fn nested_query<T: Send + Sync + 'static>(
    provider: &ServiceProvider,
) -> Result<&Repository<T>, ResolveError> {
    generic_query::<Repository<T>>(provider).await
}

pub struct OnlyDeadBranch;
// 私有、未调用函数中的查询也必须贡献闭合泛型，且跨 crate Release 构建仍一致。
async fn uncalled(provider: &ServiceProvider) {
    if false {
        let _ = nested_query::<OnlyDeadBranch>(provider).await;
    }
}

pub fn constructions() -> usize {
    CONSTRUCTIONS.load(Ordering::SeqCst)
}

pub async fn closure_query<T: Send + Sync + 'static>(
    provider: &ServiceProvider,
) -> Result<&Repository<T>, ResolveError> {
    let query = || async { provider.get_required_service::<Repository<T>>().await };
    query().await
}

#[injectable]
pub struct Indexed<const N: usize> {
    #[value(CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
}
pub async fn const_query<const N: usize>(
    provider: &ServiceProvider,
) -> Result<&Indexed<N>, ResolveError> {
    provider.get_required_service::<Indexed<N>>().await
}

pub trait SelectService {
    type Service: Send + Sync + 'static;
}
pub async fn associated_query<S: SelectService>(
    provider: &ServiceProvider,
) -> Result<&S::Service, ResolveError> {
    provider.get_required_service::<S::Service>().await
}

// 与 DI 无关的未执行泛型递归不能被查询收集器错误展开为无限类型集合。
fn unrelated_recursive<T>() {
    if std::hint::black_box(false) {
        unrelated_recursive::<Vec<T>>();
    }
}
fn unrelated_dead_branch() {
    if false {
        unrelated_recursive::<u8>();
    }
}

#[cfg(feature = "extra-root")]
struct FeatureSelected;
#[cfg(feature = "extra-root")]
async fn feature_selected_root(provider: &ServiceProvider) {
    if false {
        let _ = nested_query::<FeatureSelected>(provider).await;
    }
}

// 被 cfg 排除的调用既无需解析类型，也不贡献根。
#[cfg(any())]
async fn excluded_root(provider: &ServiceProvider) {
    let _ = provider
        .get_required_service::<UnresolvedExcludedType>()
        .await;
}

pub struct FromKnownProvider<T>(PhantomData<T>);
pub struct ThroughDeclaredProvider;

#[injectable]
pub struct Workflow<T: Send + Sync + 'static> {
    #[value(PhantomData)]
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Workflow<T> {
    pub async fn query_from_self(&self, provider: &ServiceProvider) -> usize {
        provider
            .get_required_service::<Repository<FromKnownProvider<T>>>()
            .await
            .unwrap()
            .id()
    }
}

pub trait WorkflowPort: Send + Sync {
    fn query_through_dyn<'a>(
        &'a self,
        provider: &'a ServiceProvider,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send + 'a>>;
}
impl<T: Send + Sync + 'static> WorkflowPort for Workflow<T> {
    fn query_through_dyn<'a>(
        &'a self,
        provider: &'a ServiceProvider,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send + 'a>> {
        Box::pin(async move { self.query_from_self(provider).await })
    }
}

#[injectable]
pub struct DeclaredConsumer {
    // Workflow 的闭合实例仅通过字段到达，入口只请求 trait；typed helper 的具体
    // 方法调用不会直接出现在应用 HIR。它的方法查询仍须在 build 之前进入计划。
    #[inject]
    workflow: Workflow<ThroughDeclaredProvider>,
}

pub struct FromDefault<T: ?Sized>(PhantomData<T>);
pub trait DefaultWorkflowPort: Send + Sync + 'static {
    fn default_query<'a>(
        &'a self,
        provider: &'a ServiceProvider,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = usize> + Send + 'a>> {
        Box::pin(async move {
            provider
                .get_required_service::<Repository<FromDefault<Self>>>()
                .await
                .unwrap()
                .id()
        })
    }
}
impl<T: Send + Sync + 'static> DefaultWorkflowPort for Workflow<T> {}

// 私有 static 和 helper 仅由未闭合泛型的构造代码引用；新普通查询在下游首次提供
// 实参时，上游产物仍必须保留这些真实符号，不能只保存 callback MIR。
static SECRET_COUNTER: AtomicUsize = AtomicUsize::new(707);
fn next_secret() -> usize {
    SECRET_COUNTER.fetch_add(1, Ordering::SeqCst)
}
#[injectable]
pub struct SecretRepository<T: Send + Sync + 'static> {
    #[value(next_secret())]
    secret: usize,
    #[value(PhantomData)]
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> SecretRepository<T> {
    pub fn secret(&self) -> usize {
        self.secret
    }
}
