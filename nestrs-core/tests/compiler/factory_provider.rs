use crate::activation::{
    InputSlot,
    adapter::{CleanupFuture, FactoryInvoker},
};
use crate::graph::Constructor;
use crate::lifetime::ServiceLifetime;
use crate::service::{ServiceIdentifier, ServiceKey, ServiceType};
use nestrs as declarations;
use nestrs::{factory, injectable, primary};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll, Waker},
};

struct DirectService;
struct ResultService;
struct FailedService;
struct AsyncService;
struct ExplicitFutureService;
struct ExplicitFutureResultService;
struct ConfiguredService;
struct TransientFactoryService;
struct ParameterizedService;
struct PrimaryBeforeFactoryService;
struct FactoryBeforePrimaryService;
struct QualifiedPrimaryBeforeFactoryService;
struct AliasedCratePrimaryBeforeFactoryService;
struct QualifiedFactoryBeforePrimaryService;
struct AliasedCrateFactoryBeforePrimaryService;

struct Database;
struct Cache;
struct Audit;

// factory 参数既是生成签名的一部分，也是完整编译计划中的必选边。
// 保持 factory-only 类型，验证不依赖 injectable 定义也能完成计划编译。
#[factory]
fn database() -> Database {
    Database
}

#[factory]
fn cache() -> Cache {
    Cache
}

#[derive(Debug)]
struct FactoryError;

#[factory]
fn direct_factory() -> DirectService {
    DirectService
}

#[factory]
fn result_factory() -> Result<ResultService, FactoryError> {
    Ok(ResultService)
}

#[factory]
fn failed_result_factory() -> Result<FailedService, FactoryError> {
    Err(FactoryError)
}

#[factory]
async fn async_factory() -> AsyncService {
    AsyncService
}

#[factory]
fn explicit_future_factory() -> impl ::core::future::Future<Output = ExplicitFutureService> {
    ::core::future::ready(ExplicitFutureService)
}

#[factory]
fn explicit_result_future_factory()
-> impl ::core::future::Future<Output = Result<ExplicitFutureResultService, FactoryError>> {
    ::core::future::ready(Ok(ExplicitFutureResultService))
}

async fn configured_cleanup() {}

#[factory(lifetime = Scoped, key = "configured", cleanup = "configured_cleanup")]
fn configured_factory() -> ConfiguredService {
    ConfiguredService
}

#[factory(lifetime = Transient)]
fn transient_factory() -> TransientFactoryService {
    TransientFactoryService
}

#[factory]
fn parameterized_factory(
    database: Database,
    #[inject] cache: Cache,
    #[inject("audit")] audit: Option<Audit>,
) -> ParameterizedService {
    let _ = (database, cache, audit);
    ParameterizedService
}

// `primary` 先展开时必须把 marker 留给 factory；factory 最终只能写入一项 provider。
#[primary]
#[factory]
fn primary_before_factory() -> PrimaryBeforeFactoryService {
    PrimaryBeforeFactoryService
}

// `factory` 先展开时必须直接消费下方 primary，而不让 primary 再产生单独的展开结果。
#[factory]
#[primary]
fn factory_before_primary() -> FactoryBeforePrimaryService {
    FactoryBeforePrimaryService
}

// Qualified paths and crate aliases retain the same primary handoff. Include
// borrowed parameters so this also checks the final factory signature rewrite.
#[primary]
#[nestrs::factory]
async fn qualified_primary_before_factory(
    database: Database,
) -> QualifiedPrimaryBeforeFactoryService {
    let _ = database;
    QualifiedPrimaryBeforeFactoryService
}

#[declarations::primary]
#[declarations::factory]
fn aliased_crate_primary_before_factory(
    database: Database,
) -> AliasedCratePrimaryBeforeFactoryService {
    let _ = database;
    AliasedCratePrimaryBeforeFactoryService
}

#[nestrs::factory]
#[nestrs::primary]
fn qualified_factory_before_primary() -> QualifiedFactoryBeforePrimaryService {
    QualifiedFactoryBeforePrimaryService
}

#[declarations::factory]
#[declarations::primary]
fn aliased_crate_factory_before_primary() -> AliasedCrateFactoryBeforePrimaryService {
    AliasedCrateFactoryBeforePrimaryService
}

#[injectable]
struct Secondary;
trait PrimaryBefore: Send + Sync {}
impl PrimaryBefore for PrimaryBeforeFactoryService {}
impl PrimaryBefore for Secondary {}

trait FactoryBefore: Send + Sync {}
impl FactoryBefore for FactoryBeforePrimaryService {}
impl FactoryBefore for Secondary {}

trait QualifiedPrimaryBefore: Send + Sync {}
impl QualifiedPrimaryBefore for QualifiedPrimaryBeforeFactoryService {}
impl QualifiedPrimaryBefore for Secondary {}

trait AliasedPrimaryBefore: Send + Sync {}
impl AliasedPrimaryBefore for AliasedCratePrimaryBeforeFactoryService {}
impl AliasedPrimaryBefore for Secondary {}

trait QualifiedFactoryBefore: Send + Sync {}
impl QualifiedFactoryBefore for QualifiedFactoryBeforePrimaryService {}
impl QualifiedFactoryBefore for Secondary {}

trait AliasedFactoryBefore: Send + Sync {}
impl AliasedFactoryBefore for AliasedCrateFactoryBeforePrimaryService {}
impl AliasedFactoryBefore for Secondary {}

#[allow(dead_code)]
async fn primary_queries(provider: &crate::ServiceProvider) {
    let _ = provider.get_service::<dyn PrimaryBefore>().await;
    let _ = provider.get_service::<dyn FactoryBefore>().await;
    let _ = provider.get_service::<dyn QualifiedPrimaryBefore>().await;
    let _ = provider.get_service::<dyn AliasedPrimaryBefore>().await;
    let _ = provider.get_service::<dyn QualifiedFactoryBefore>().await;
    let _ = provider.get_service::<dyn AliasedFactoryBefore>().await;
}

fn complete_immediately<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + 'static>>) -> T {
    let mut context = Context::from_waker(Waker::noop());
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("测试 cleanup 没有挂起点"),
    }
}

fn node<T: Send + Sync + 'static>() -> &'static crate::graph::CompiledNode {
    crate::graph::plan::load()
        .graph
        .nodes
        .iter()
        .find(|node| node.identifier.service_type == ServiceType::create::<T>())
        .unwrap()
}

#[test]
fn factory_compiles_configuration_and_parameter_inputs() {
    let graph = &crate::graph::plan::load().graph;
    let configured = node::<ConfiguredService>();
    assert_eq!(
        configured.identifier,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("configured".into())),
            ServiceType::create::<ConfiguredService>()
        )
    );
    assert_eq!(configured.common.lifetime, ServiceLifetime::Scoped);
    assert!(
        configured
            .common
            .source
            .file
            .ends_with("factory_provider.rs")
    );
    assert!(matches!(
        configured.constructor,
        Constructor::Factory(FactoryInvoker::Sync(_))
    ));
    let cleanup: CleanupFuture = configured.common.cleanup.expect("cleanup")();
    complete_immediately(cleanup);
    assert!(configured.dependencies.is_empty());

    let transient = node::<TransientFactoryService>();
    assert_eq!(transient.common.lifetime, ServiceLifetime::Transient);
    assert!(transient.dependencies.is_empty());
    assert!(matches!(
        transient.constructor,
        Constructor::Factory(FactoryInvoker::Sync(_))
    ));

    let parameterized = node::<ParameterizedService>();
    assert_eq!(parameterized.dependencies.len(), 3);
    for (slot, (service_type, label)) in [
        (ServiceType::create::<Database>(), "database"),
        (ServiceType::create::<Cache>(), "cache"),
    ]
    .into_iter()
    .enumerate()
    {
        let input = &parameterized.dependencies[slot];
        assert_eq!(input.slot, InputSlot::new(slot));
        assert_eq!(input.label, Some(label));
        assert_eq!(input.requested, ServiceIdentifier::from(service_type));
        assert!(!input.optional);
        assert_eq!(
            graph.nodes[input.input.target().unwrap()].identifier,
            input.requested
        );
    }
    let audit = &parameterized.dependencies[2];
    assert_eq!(audit.slot, InputSlot::new(2));
    assert_eq!(audit.label, Some("audit"));
    assert_eq!(
        audit.requested,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("audit".into())),
            ServiceType::create::<Audit>()
        )
    );
    assert!(audit.optional);
    assert!(audit.input.target().is_none());
    let crate::graph::DependencyInput::Absent(crate::graph::AbsentInput::Immediate(prepare)) =
        audit.input
    else {
        panic!("缺席的 factory 普通输入必须交付 Option<Injection<T>>")
    };
    assert!(
        prepare(audit.slot, None)
            .unwrap()
            .into_optional::<Audit>(audit.slot)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn factory_invokers_construct_all_supported_return_shapes_and_report_failure() {
    for node in [
        node::<DirectService>(),
        node::<ResultService>(),
        node::<FailedService>(),
    ] {
        assert!(matches!(
            node.constructor,
            Constructor::Factory(FactoryInvoker::Sync(_))
        ));
    }
    for node in [
        node::<AsyncService>(),
        node::<ExplicitFutureService>(),
        node::<ExplicitFutureResultService>(),
    ] {
        assert!(matches!(
            node.constructor,
            Constructor::Factory(FactoryInvoker::Async(_))
        ));
    }
    let provider = crate::ServiceProvider::build().await.unwrap();
    provider
        .get_required_service::<DirectService>()
        .await
        .unwrap();
    provider
        .get_required_service::<ResultService>()
        .await
        .unwrap();
    provider
        .get_required_service::<AsyncService>()
        .await
        .unwrap();
    provider
        .get_required_service::<ExplicitFutureService>()
        .await
        .unwrap();
    provider
        .get_required_service::<ExplicitFutureResultService>()
        .await
        .unwrap();
    provider
        .get_required_service::<ParameterizedService>()
        .await
        .unwrap();
    let error = provider
        .get_required_service::<FailedService>()
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("FactoryError"));
    provider.dispose_async().await.unwrap();
}

#[test]
fn factory_primary_order_and_aliases_select_one_real_execution_target() {
    let graph = &crate::graph::plan::load().graph;
    for (port, service_type) in [
        (
            ServiceType::create::<dyn PrimaryBefore>(),
            ServiceType::create::<PrimaryBeforeFactoryService>(),
        ),
        (
            ServiceType::create::<dyn FactoryBefore>(),
            ServiceType::create::<FactoryBeforePrimaryService>(),
        ),
        (
            ServiceType::create::<dyn QualifiedPrimaryBefore>(),
            ServiceType::create::<QualifiedPrimaryBeforeFactoryService>(),
        ),
        (
            ServiceType::create::<dyn AliasedPrimaryBefore>(),
            ServiceType::create::<AliasedCratePrimaryBeforeFactoryService>(),
        ),
        (
            ServiceType::create::<dyn QualifiedFactoryBefore>(),
            ServiceType::create::<QualifiedFactoryBeforePrimaryService>(),
        ),
        (
            ServiceType::create::<dyn AliasedFactoryBefore>(),
            ServiceType::create::<AliasedCrateFactoryBeforePrimaryService>(),
        ),
    ] {
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.identifier.service_type == service_type)
                .count(),
            1
        );
        let route = &graph.routes[&ServiceIdentifier::from(port)];
        assert!(route.projection.is_some());
        assert_eq!(
            graph.nodes[route.provider].identifier.service_type,
            service_type
        );
        assert!(matches!(
            graph.nodes[route.provider].constructor,
            Constructor::Factory(_)
        ));
    }
}
