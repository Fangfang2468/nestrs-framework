//! 安全的手写 adapter 可以截留输入；隐藏 ABI 也必须真实保活 token 的实例。
use nestrs::injectable;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

use nestrs_core::{
    __private::{
        ClassProvider, ConstructionError, ConstructionInputs, Delivery, DependencyRequest,
        ErasedService, Injection, InputSlot, Provider, ProviderCommon, ProviderSource,
        ServiceIdentifier, ServiceLifetime, ServiceSource, ServiceType, prepare_required,
    },
    ServiceProvider,
};

static DEPENDENCY_DROPS: AtomicUsize = AtomicUsize::new(0);
static DEPENDENCY_CLEANUPS: AtomicUsize = AtomicUsize::new(0);
static ESCAPED: Mutex<Option<Injection<Dependency>>> = Mutex::new(None);

async fn cleanup_dependency() {
    DEPENDENCY_CLEANUPS.fetch_add(1, Ordering::SeqCst);
}

#[injectable(cleanup = "cleanup_dependency")]
struct Dependency {
    #[value(91)]
    value: u32,
}

impl Drop for Dependency {
    fn drop(&mut self) {
        DEPENDENCY_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

struct Consumer;

fn construct_consumer(mut inputs: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    let dependency = inputs.take::<Dependency>(InputSlot::new(0))?;
    inputs.ensure_all_consumed()?;
    *ESCAPED.lock().unwrap() = Some(dependency);
    Ok(ErasedService::new(Consumer))
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn consumer_provider() -> Provider {
    Provider::Class(ClassProvider {
        provide: ServiceIdentifier::from(ServiceType::create::<Consumer>()),
        common: ProviderCommon {
            lifetime: ServiceLifetime::Singleton,
            primary: false,
            source: ServiceSource::new(file!(), line!(), column!()),
            cleanup: None,
        },
        dependencies: vec![DependencyRequest {
            declaration_position: 0,
            input_slot: InputSlot::new(0),
            token: ServiceIdentifier::from(ServiceType::create::<Dependency>()),
            optional: false,
            label: Some("captured_dependency"),
            delivery: Delivery::Direct(prepare_required::<Dependency>),
            provider_source: ProviderSource::Registered,
        }],
        constructor: construct_consumer,
    })
}

#[tokio::test]
async fn safe_adapter_escape_survives_disposal_and_drops_only_after_the_final_token() {
    let provider = ServiceProvider::build().await.unwrap();
    nestrs_core::get_required_service!(provider, Consumer)
        .await
        .unwrap();
    assert_eq!(DEPENDENCY_DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(DEPENDENCY_CLEANUPS.load(Ordering::SeqCst), 0);

    provider.dispose_async().await.unwrap();
    assert_eq!(DEPENDENCY_CLEANUPS.load(Ordering::SeqCst), 1);
    assert_eq!(DEPENDENCY_DROPS.load(Ordering::SeqCst), 0);

    let escaped = ESCAPED.lock().unwrap().take().unwrap();
    assert_eq!(escaped.value, 91);
    drop(escaped);
    assert_eq!(DEPENDENCY_DROPS.load(Ordering::SeqCst), 1);
    assert_eq!(DEPENDENCY_CLEANUPS.load(Ordering::SeqCst), 1);
}
