use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Wake, Waker},
};

use nestrs_core::{
    __private::{CleanupFuture, FactoryInvoker, InjectionTarget, Provider, REFLECTED_PROVIDERS},
    lifetime::Lifetime,
    registration::{
        service_identifier::ServiceIdentifier, service_key::ServiceKey, service_type::ServiceType,
    },
};
use nestrs_macro::{factory, primary};

struct DirectService;
struct ResultService;
struct FailedService;
struct AsyncService;
struct ExplicitFutureService;
struct ExplicitFutureResultService;
struct ConfiguredService;
struct ParameterizedService;
struct PrimaryBeforeFactoryService;
struct FactoryBeforePrimaryService;

struct Database;
struct Cache;
struct Audit;

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
    async { ExplicitFutureService }
}

#[factory]
fn explicit_result_future_factory()
-> impl ::core::future::Future<Output = Result<ExplicitFutureResultService, FactoryError>> {
    async { Ok(ExplicitFutureResultService) }
}

async fn configured_cleanup() {}

#[factory(lifetime = Scoped, key = "configured", cleanup = "configured_cleanup")]
fn configured_factory() -> ConfiguredService {
    ConfiguredService
}

#[factory]
fn parameterized_factory(
    database: Database,
    #[inject] cache: Cache,
    #[inject(key = "audit")] audit: Option<Audit>,
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

struct NoopWake;

impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}

/// 当前 factory 适配器只包裹无 await 的测试函数，故它们首次 poll 就应完成。这里不用
/// runtime，以免把 activation runtime 的实现误作为本次 provider ABI 的前提。
fn complete_immediately<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + 'static>>) -> T {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);

    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test factory future should complete without an executor"),
    }
}

fn providers() -> Vec<Provider> {
    REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .collect()
}

fn factory_provider_for<T>() -> Provider
where
    T: Send + Sync + 'static,
{
    providers()
        .into_iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Factory { provide, .. }
                    if provide.service_type == ServiceType::create::<T>()
            )
        })
        .unwrap_or_else(|| {
            panic!(
                "factory provider should be registered for {}",
                std::any::type_name::<T>()
            )
        })
}

#[test]
fn factory_collects_common_configuration_and_parameter_injections() {
    let configured = factory_provider_for::<ConfiguredService>();
    let Provider::Factory {
        provide,
        common,
        dependencies,
        invoker,
    } = configured
    else {
        panic!("configured factory should register Provider::Factory");
    };

    assert_eq!(
        provide,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("configured")),
            ServiceType::create::<ConfiguredService>(),
        )
    );
    assert_eq!(common.lifetime, Lifetime::Scoped);
    assert!(!common.primary);
    assert!(common.source.file.ends_with("factory_provider.rs"));
    assert!(matches!(invoker, FactoryInvoker::Sync(_)));

    let cleanup = common
        .cleanup
        .expect("configured factory should retain cleanup hook");
    let cleanup_future: CleanupFuture = cleanup();
    complete_immediately(cleanup_future);
    assert!(dependencies.is_empty());

    let parameterized = factory_provider_for::<ParameterizedService>();
    let Provider::Factory { dependencies, .. } = parameterized else {
        panic!("parameterized factory should register Provider::Factory");
    };
    assert_eq!(dependencies.len(), 3);

    let database = dependencies[0];
    assert_eq!(database.declaration_position, 0);
    assert_eq!(database.input_position.0, 0);
    assert_eq!(database.label, Some("database"));
    assert_eq!(
        database.token,
        ServiceIdentifier::from(ServiceType::create::<Database>())
    );
    assert!(!database.optional);
    assert_eq!(database.target, InjectionTarget::Concrete);
    assert!(database.prepare_input.is_some());
    assert!(database.closed_provider.is_none());

    let cache = dependencies[1];
    assert_eq!(cache.declaration_position, 1);
    assert_eq!(cache.input_position.0, 1);
    assert_eq!(cache.label, Some("cache"));
    assert_eq!(
        cache.token,
        ServiceIdentifier::from(ServiceType::create::<Cache>())
    );
    assert!(!cache.optional);
    assert_eq!(cache.target, InjectionTarget::Concrete);
    assert!(cache.prepare_input.is_some());

    let audit = dependencies[2];
    assert_eq!(audit.declaration_position, 2);
    assert_eq!(audit.input_position.0, 2);
    assert_eq!(audit.label, Some("audit"));
    assert_eq!(
        audit.token,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("audit")),
            ServiceType::create::<Audit>(),
        )
    );
    assert!(audit.optional);
    assert_eq!(audit.target, InjectionTarget::Concrete);
    assert!(audit.prepare_input.is_some());
    assert!(audit.closed_provider.is_none());
}

#[test]
fn factory_invokers_describe_all_supported_return_shapes() {
    let direct = factory_provider_for::<DirectService>();
    let Provider::Factory { invoker, .. } = direct else {
        panic!("direct factory should register Provider::Factory");
    };
    assert!(matches!(invoker, FactoryInvoker::Sync(_)));

    let result = factory_provider_for::<ResultService>();
    let Provider::Factory { invoker, .. } = result else {
        panic!("result factory should register Provider::Factory");
    };
    assert!(matches!(invoker, FactoryInvoker::Sync(_)));

    let failed = factory_provider_for::<FailedService>();
    let Provider::Factory { invoker, .. } = failed else {
        panic!("failing result factory should register Provider::Factory");
    };
    assert!(matches!(invoker, FactoryInvoker::Sync(_)));

    let asynchronous = factory_provider_for::<AsyncService>();
    let Provider::Factory { invoker, .. } = asynchronous else {
        panic!("async factory should register Provider::Factory");
    };
    assert!(matches!(invoker, FactoryInvoker::Async(_)));

    let explicit_future = factory_provider_for::<ExplicitFutureService>();
    let Provider::Factory { invoker, .. } = explicit_future else {
        panic!("explicit Future factory should register Provider::Factory");
    };
    assert!(matches!(invoker, FactoryInvoker::Async(_)));

    let explicit_result_future = factory_provider_for::<ExplicitFutureResultService>();
    let Provider::Factory { invoker, .. } = explicit_result_future else {
        panic!("explicit Result Future factory should register Provider::Factory");
    };
    assert!(matches!(invoker, FactoryInvoker::Async(_)));
}

#[test]
fn factory_consumes_primary_in_either_attribute_order_once() {
    let providers = providers();

    for service_type in [
        ServiceType::create::<PrimaryBeforeFactoryService>(),
        ServiceType::create::<FactoryBeforePrimaryService>(),
    ] {
        let matching: Vec<_> = providers
            .iter()
            .filter(|provider| {
                matches!(
                    provider,
                    Provider::Factory { provide, .. } if provide.service_type == service_type
                )
            })
            .collect();
        assert_eq!(
            matching.len(),
            1,
            "each primary factory should emit exactly one Provider::Factory"
        );

        let Provider::Factory { common, .. } = matching[0] else {
            unreachable!("the filter only retains factory providers");
        };
        assert!(common.primary);
    }
}
