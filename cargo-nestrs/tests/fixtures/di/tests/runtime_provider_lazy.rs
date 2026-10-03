//! 服务声明级预热策略经真实工具链、闭合泛型、绑定与运行时门面的集成回归。
//! 各用例共享声明，独立 root 的构造记录由测试锁隔离。
use nestrs::{factory, injectable, lazy, primary};
use nestrs_core::{
    InitializationMode, ServiceKey, ServiceProvider, ServiceProviderOptions, ServiceScopeOptions,
};
use std::{marker::PhantomData, sync::Mutex};

static TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static CREATED: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn created(name: &'static str) -> usize {
    let mut values = CREATED.lock().unwrap();
    let id = values.iter().filter(|value| **value == name).count();
    values.push(name);
    id
}
fn count(name: &str) -> usize {
    CREATED
        .lock()
        .unwrap()
        .iter()
        .filter(|value| **value == name)
        .count()
}

#[injectable]
struct Inherited {
    #[value(created("inherited"))]
    _id: usize,
}

#[lazy]
#[injectable]
struct Deferred {
    #[value(created("deferred"))]
    id: usize,
}

#[injectable]
#[lazy(false)]
struct Forced {
    #[value(created("forced"))]
    id: usize,
}

struct SyncClient;
#[lazy(false)]
#[factory]
fn sync_client() -> SyncClient {
    created("sync");
    SyncClient
}

struct AsyncClient;
#[factory]
#[lazy(false)]
async fn async_client() -> AsyncClient {
    tokio::task::yield_now().await;
    created("async");
    AsyncClient
}

#[lazy(true)]
#[injectable]
struct RequiredDespiteLazy {
    #[value(created("required-lazy"))]
    _id: usize,
}

#[lazy(false)]
#[injectable]
struct StartupConsumer {
    #[inject]
    _required: RequiredDespiteLazy,
    #[inject]
    #[lazy]
    deferred: Deferred,
}

struct Customer;
#[injectable]
#[lazy(false)]
struct Cache<T> {
    marker: PhantomData<T>,
    #[value(created("cache"))]
    id: usize,
}

trait Channel: Send + Sync {
    fn name(&self) -> &'static str;
}
struct MainChannel;
impl Channel for MainChannel {
    fn name(&self) -> &'static str {
        "primary"
    }
}
struct BackupChannel;
impl Channel for BackupChannel {
    fn name(&self) -> &'static str {
        "backup"
    }
}

#[primary]
#[lazy(false)]
#[factory(key = "mail")]
fn main_channel() -> MainChannel {
    created("main-channel");
    MainChannel
}

#[factory(key = "mail")]
#[lazy]
fn backup_channel() -> BackupChannel {
    created("backup-channel");
    BackupChannel
}

#[lazy(true)]
#[factory(key = 7)]
fn indexed_channel() -> BackupChannel {
    created("indexed-channel");
    BackupChannel
}

#[injectable(lifetime = Scoped)]
struct ScopedInherited {
    #[value(created("scoped-inherited"))]
    _id: usize,
}

#[lazy(false)]
#[injectable(lifetime = Scoped)]
struct ScopedForced {
    #[value(created("scoped-forced"))]
    id: usize,
}

#[injectable(lifetime = Scoped)]
#[lazy(true)]
struct ScopedDeferred {
    #[value(created("scoped-deferred"))]
    id: usize,
}

#[injectable(lifetime = Transient)]
#[lazy(false)]
struct TransientForced {
    #[value(created("transient-forced"))]
    id: usize,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provider_overrides_select_roots_without_changing_field_or_lifetime_semantics() {
    let _test = TEST_LOCK.lock().await;
    for mode in [InitializationMode::Lazy, InitializationMode::Eager] {
        CREATED.lock().unwrap().clear();
        let provider = ServiceProvider::build(Some(ServiceProviderOptions {
            initialization: mode,
            ..Default::default()
        }))
        .await
        .unwrap();
        assert_eq!(
            count("inherited"),
            usize::from(mode == InitializationMode::Eager)
        );
        for name in [
            "forced",
            "sync",
            "async",
            "required-lazy",
            "cache",
            "main-channel",
        ] {
            assert_eq!(
                count(name),
                1,
                "显式立即策略或普通依赖必须在 build 返回前构造 {name}"
            );
        }
        for name in [
            "deferred",
            "backup-channel",
            "indexed-channel",
            "transient-forced",
            "scoped-forced",
            "scoped-inherited",
            "scoped-deferred",
        ] {
            assert_eq!(count(name), 0, "{name} 不能被错误选为启动根");
        }
        assert_eq!(
            provider.get_required_service::<Forced>().await.unwrap().id,
            0
        );
        // 查询方法贡献闭合 Cache<Customer> 根；泛型蓝图的服务策略必须传到闭合实例。
        assert_eq!(
            provider
                .get_required_service::<Cache<Customer>>()
                .await
                .unwrap()
                .id,
            0
        );
        let channel = provider
            .get_required_keyed_service::<dyn Channel>(ServiceKey::Named("mail".into()))
            .await
            .unwrap();
        assert_eq!(channel.name(), "primary");
        assert_eq!(count("backup-channel"), 0);
        assert_eq!(
            provider
                .get_required_keyed_service::<dyn Channel>(ServiceKey::Indexed(7))
                .await
                .unwrap()
                .name(),
            "backup"
        );
        assert_eq!(count("indexed-channel"), 1);
        let consumer = provider
            .get_required_service::<StartupConsumer>()
            .await
            .unwrap();
        assert_eq!(count("deferred"), 0);
        assert_eq!(consumer.deferred.get().await.unwrap().id, 0);
        assert_eq!(count("deferred"), 1);
        assert!(std::ptr::eq(
            consumer.deferred.get().await.unwrap(),
            provider.get_required_service::<Deferred>().await.unwrap()
        ));
        let first = provider
            .get_required_service::<TransientForced>()
            .await
            .unwrap();
        let second = provider
            .get_required_service::<TransientForced>()
            .await
            .unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(count("transient-forced"), 2);
        provider.dispose_async().await.unwrap();
    }
}

#[tokio::test]
async fn scope_creation_applies_independent_defaults_overrides_and_service_policies() {
    let _test = TEST_LOCK.lock().await;
    for root_mode in [InitializationMode::Lazy, InitializationMode::Eager] {
        for scope_mode in [InitializationMode::Lazy, InitializationMode::Eager] {
            CREATED.lock().unwrap().clear();
            let provider = ServiceProvider::build(Some(ServiceProviderOptions {
                initialization: root_mode,
                scope_initialization: scope_mode,
                ..Default::default()
            }))
            .await
            .unwrap();
            for name in ["scoped-inherited", "scoped-forced", "scoped-deferred"] {
                assert_eq!(count(name), 0, "build 不提前创建 Scoped 服务");
            }

            let first = provider.create_scope(None).await.unwrap();
            assert_eq!(
                count("scoped-inherited"),
                usize::from(scope_mode == InitializationMode::Eager)
            );
            assert_eq!(
                count("scoped-forced"),
                1,
                "服务级立即策略在 Lazy scope 下也生效"
            );
            assert_eq!(count("scoped-deferred"), 0);

            let override_mode = match scope_mode {
                InitializationMode::Lazy => InitializationMode::Eager,
                InitializationMode::Eager => InitializationMode::Lazy,
            };
            let second = provider
                .create_scope(Some(ServiceScopeOptions {
                    initialization: override_mode,
                }))
                .await
                .unwrap();
            assert_eq!(
                count("scoped-inherited"),
                1,
                "单次覆盖与容器默认分别作用于各自 scope"
            );
            assert_eq!(count("scoped-forced"), 2);
            assert_eq!(count("scoped-deferred"), 0);

            let first_service = first
                .service_provider()
                .get_required_service::<ScopedForced>()
                .await
                .unwrap();
            let first_again = first
                .service_provider()
                .get_required_service::<ScopedForced>()
                .await
                .unwrap();
            let second_service = second
                .service_provider()
                .get_required_service::<ScopedForced>()
                .await
                .unwrap();
            assert!(std::ptr::eq(first_service, first_again));
            assert_ne!(first_service.id, second_service.id);
            assert_eq!(count("scoped-forced"), 2, "查询复用已初始化的 Scoped 实例");
            assert_eq!(
                first
                    .service_provider()
                    .get_required_service::<ScopedDeferred>()
                    .await
                    .unwrap()
                    .id,
                0
            );
            assert_eq!(count("scoped-deferred"), 1);
            assert_eq!(count("transient-forced"), 0, "Transient 不成为自主初始化根");

            // 单次覆盖不写回默认，后续普通创建仍使用 provider 的 scope 策略。
            let third = provider.create_scope(None).await.unwrap();
            assert_eq!(
                count("scoped-inherited"),
                1 + usize::from(scope_mode == InitializationMode::Eager)
            );
            assert_eq!(count("scoped-forced"), 3);
            assert_eq!(count("scoped-deferred"), 1);
            first.dispose_async().await.unwrap();
            second.dispose_async().await.unwrap();
            third.dispose_async().await.unwrap();
            provider.dispose_async().await.unwrap();
        }
    }
}
