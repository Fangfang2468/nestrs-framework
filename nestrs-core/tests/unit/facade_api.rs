use crate as nestrs_core;
use ahash::AHashMap;
use nestrs_core::{
    BuildError, DisposeError, InitializationMode, ResolveError, ServiceKey, ServiceProvider,
    ServiceProviderOptions, ServiceScope,
};
use std::{
    error::Error,
    num::NonZeroUsize,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use crate::{
    ServiceLifetime,
    activation::{DependencyLease, ErasedService, ReleaseDomain, ServiceProjector, project_bound},
    graph::{CompiledNode, Constructor, NodePolicy, RootRoute, ValidatedGraph},
    runtime::Runtime,
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};

#[derive(Debug)]
struct Concrete;
trait Port: Send + Sync {}
fn assert_error<E: Error>() {}

fn provider_api(provider: &ServiceProvider) {
    std::mem::drop(ServiceProvider::build());
    std::mem::drop(ServiceProvider::build_with_options(
        ServiceProviderOptions {
            initialization: InitializationMode::Eager,
            max_concurrent_activations: NonZeroUsize::new(4).unwrap(),
        },
    ));
    std::mem::drop(provider.get_required_service::<Concrete>());
    std::mem::drop(provider.get_service::<Concrete>());
    std::mem::drop(
        provider.get_required_keyed_service::<Concrete>(ServiceKey::Named("primary".to_owned())),
    );
    std::mem::drop(provider.get_keyed_service::<Concrete>(ServiceKey::Indexed(1)));
    std::mem::drop(provider.get_required_service::<dyn Port>());
    let _ = provider.create_scope();
}
async fn chained_scope_api(scope: &ServiceScope<'_>) {
    let value = scope
        .service_provider()
        .get_required_service::<Concrete>()
        .await
        .unwrap();
    let _ = std::ptr::from_ref(value);
    std::mem::drop(scope.warm_up());
}
fn provider_disposal(provider: ServiceProvider) {
    std::mem::drop(provider.dispose_async());
}
fn scope_disposal(scope: ServiceScope<'_>) {
    std::mem::drop(scope.dispose_async());
}

#[test]
fn public_facade_types_and_signatures_are_available() {
    let _ = (
        provider_api,
        chained_scope_api,
        provider_disposal,
        scope_disposal,
    );
    assert_error::<BuildError>();
    assert_error::<ResolveError>();
    assert_error::<DisposeError>();
    assert_eq!(
        ServiceProviderOptions::default().initialization,
        InitializationMode::Lazy
    );
    assert_eq!(
        ServiceProviderOptions::default()
            .max_concurrent_activations
            .get(),
        32
    );
}

#[tokio::test]
async fn empty_provider_and_scope_complete_full_lifecycle() {
    let provider = ServiceProvider::build().await.unwrap();
    assert!(provider.get_service::<Concrete>().await.unwrap().is_none());
    assert!(
        provider
            .get_required_service::<Concrete>()
            .await
            .unwrap_err()
            .to_string()
            .contains("未注册")
    );
    let scope = provider.create_scope();
    scope.warm_up().await.unwrap();
    assert!(
        scope
            .service_provider()
            .get_service::<dyn Port>()
            .await
            .unwrap()
            .is_none()
    );
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}

#[test]
fn build_reports_missing_runtime_without_creating_one() {
    use std::{
        future::Future,
        task::{Context, Poll, Waker},
    };
    let mut build = std::pin::pin!(ServiceProvider::build());
    let mut context = Context::from_waker(Waker::noop());
    assert!(matches!(
        build.as_mut().poll(&mut context),
        Poll::Ready(Err(BuildError::RuntimeUnavailable))
    ));
}

/// 每个实例有独立析构计数器。测试保留计数器，不保留服务自身的 lease，因此能观察
/// scope/owner journal 是否真的承担引用保活，也不会受并行运行的其他测试影响。
struct ObservedService(Arc<AtomicUsize>);

trait ObservedPort: Send + Sync {
    fn identity(&self) -> usize;
    fn drops(&self) -> Arc<AtomicUsize>;
}

impl ObservedPort for ObservedService {
    fn identity(&self) -> usize {
        std::ptr::from_ref(self) as usize
    }

    fn drops(&self) -> Arc<AtomicUsize> {
        self.0.clone()
    }
}

impl Drop for ObservedService {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn observed_projection() -> ServiceProjector {
    |slot, input, output| {
        project_bound::<ObservedService, dyn ObservedPort>(slot, input, output, |value| value)
    }
}

/// 直接建立本测试的冻结计划，再走真实 runtime 与公开门面；不修改进程共享的
/// OnceLock，也不为应用添加可替换计划的入口。两种 key 各有一个无依赖 provider。
fn observed_provider(lifetime: ServiceLifetime, project: ServiceProjector) -> ServiceProvider {
    let mut nodes = Vec::new();
    let mut routes = AHashMap::new();
    for key in [None, Some(ServiceKey::Named("named".to_owned()))] {
        let provider = nodes.len();
        let identifier =
            ServiceIdentifier::new(key.clone(), ServiceType::create::<ObservedService>());
        routes.insert(
            identifier.clone(),
            RootRoute {
                provider,
                projection: None,
            },
        );
        routes.insert(
            ServiceIdentifier::new(key, ServiceType::create::<dyn ObservedPort>()),
            RootRoute {
                provider,
                projection: Some(project),
            },
        );
        nodes.push(CompiledNode {
            identifier,
            common: NodePolicy {
                lifetime,
                lazy: None,
                source: ServiceSource::new(file!(), line!(), column!()),
                cleanup: None,
            },
            dependencies: vec![],
            constructor: Constructor::Class(|inputs| {
                inputs.ensure_all_consumed()?;
                Ok(ErasedService::new(ObservedService(Arc::new(
                    AtomicUsize::new(0),
                ))))
            }),
            requires_scope: lifetime == ServiceLifetime::Scoped,
        });
    }
    let graph = Arc::new(ValidatedGraph {
        nodes,
        routes,
        topological_order: vec![0, 1],
        dependents: vec![vec![], vec![]],
    });
    let (runtime, owner) = Runtime::start(graph.clone(), 2);
    ServiceProvider {
        graph,
        runtime,
        owner,
    }
}

#[tokio::test]
async fn chained_scope_trait_and_keyed_queries_share_the_selected_instance() {
    let provider = observed_provider(ServiceLifetime::Scoped, observed_projection());
    // 可选查询也不能把生命周期限制当成未注册；正常请求需要一个 scope。
    assert!(provider.get_service::<dyn ObservedPort>().await.is_err());
    let first = provider.create_scope();
    let second = provider.create_scope();
    let concrete = first
        .service_provider()
        .get_required_service::<ObservedService>()
        .await
        .unwrap();
    let interface = first
        .service_provider()
        .get_required_service::<dyn ObservedPort>()
        .await
        .unwrap();
    assert_eq!(interface.identity(), concrete.identity());
    let first_drops = interface.drops();

    let named = first
        .service_provider()
        .get_required_keyed_service::<dyn ObservedPort>(ServiceKey::Named("named".to_owned()))
        .await
        .unwrap();
    let optional_named = first
        .service_provider()
        .get_keyed_service::<dyn ObservedPort>(ServiceKey::Named("named".to_owned()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(named.identity(), optional_named.identity());
    assert_ne!(named.identity(), interface.identity());
    let named_drops = named.drops();
    assert!(
        first
            .service_provider()
            .get_keyed_service::<dyn ObservedPort>(ServiceKey::Indexed(99))
            .await
            .unwrap()
            .is_none()
    );
    let other_scope = second
        .service_provider()
        .get_required_service::<dyn ObservedPort>()
        .await
        .unwrap();
    assert_ne!(other_scope.identity(), interface.identity());
    let second_drops = other_scope.drops();
    // 每次查询创建的临时视图与投影 token 都已结束，引用仍由实际 scope 保活。
    assert_eq!(first_drops.load(Ordering::SeqCst), 0);
    assert_eq!(named_drops.load(Ordering::SeqCst), 0);
    first.dispose_async().await.unwrap();
    assert_eq!(first_drops.load(Ordering::SeqCst), 1);
    assert_eq!(named_drops.load(Ordering::SeqCst), 1);
    assert_eq!(second_drops.load(Ordering::SeqCst), 0);
    second.dispose_async().await.unwrap();
    assert_eq!(second_drops.load(Ordering::SeqCst), 1);
    provider.dispose_async().await.unwrap();
}

#[test]
fn projected_root_reference_is_kept_alive_after_tokio_stops() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let provider = {
        let _entered = runtime.enter();
        observed_provider(ServiceLifetime::Singleton, observed_projection())
    };
    let service = runtime
        .block_on(provider.get_required_service::<dyn ObservedPort>())
        .unwrap();
    let identity = service.identity();
    let drops = service.drops();
    drop(runtime);
    // 协调器、缓存和投影临时令牌都不能成为返回引用唯一的内存持有者。
    // 最后一次读取发生在 Tokio 停止之后，实际 root owner 仍然存活。
    assert_eq!(service.identity(), identity);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(provider);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn facade_rejects_a_projector_that_substitutes_another_instance() {
    let provider = observed_provider(ServiceLifetime::Singleton, |slot, _input, output| {
        // 没有 unsafe：这份令牌指向合法的同类型实例，但它从未被当前 owner 发布。
        // 只校验 Rust 类型会让门面错误地把它返回为 owner 借用，故必须同时核对 lease。
        let replacement = DependencyLease::new(
            ErasedService::new(ObservedService(Arc::new(AtomicUsize::new(0)))),
            vec![],
            ReleaseDomain::new(),
        );
        project_bound::<ObservedService, dyn ObservedPort>(
            slot,
            replacement.erased_ref(),
            output,
            |value| value,
        )
    });
    let error = provider
        .get_required_service::<dyn ObservedPort>()
        .await
        .err()
        .expect("不能把其他实例的 trait 地址作为当前 owner 的借用交付");
    assert!(error.to_string().contains("不同实例"));
    assert!(provider.get_service::<dyn ObservedPort>().await.is_err());
    let original = provider
        .get_required_service::<ObservedService>()
        .await
        .unwrap();
    let drops = original.drops();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    provider.dispose_async().await.unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
