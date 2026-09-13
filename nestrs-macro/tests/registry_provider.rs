//! 注册链路验证：宏生成的 `Provider` 经 linkme 收集后，能被 registry 按角色归一化。

use nestrs_core::registration::{
    registry::{Activation, Registry},
    service_identifier::ServiceIdentifier,
    service_key::ServiceKey,
    service_type::ServiceType,
};
use nestrs_macro::{bind, factory, injectable};

trait Greeter: Send + Sync {
    fn greet(&self) -> &'static str;
}

struct GreeterService;

#[injectable]
struct Repository {
    #[value("registry")]
    label: String,
}

#[factory(key = "greeting")]
fn make_greeter() -> GreeterService {
    GreeterService
}

#[bind]
impl Greeter for GreeterService {
    fn greet(&self) -> &'static str {
        "hello"
    }
}

#[test]
fn registry_partitions_macro_generated_registrations() {
    let registry = Registry::from_reflected();

    let repository =
        registry.providers(ServiceIdentifier::from(ServiceType::create::<Repository>()));
    assert_eq!(repository.len(), 1);
    assert!(matches!(repository[0].activation, Activation::Class(_)));
    assert!(repository[0].dependencies.is_empty());
    assert!(repository[0].common.cleanup.is_none());

    let greeter = registry.providers(ServiceIdentifier::new(
        Some(ServiceKey::Named("greeting")),
        ServiceType::create::<GreeterService>(),
    ));
    assert_eq!(greeter.len(), 1);
    assert!(matches!(greeter[0].activation, Activation::SyncFactory(_)));

    let bindings = registry.bindings(ServiceType::create::<dyn Greeter>());
    assert_eq!(bindings.len(), 1);
    assert_eq!(
        bindings[0].concrete_type,
        ServiceType::create::<GreeterService>()
    );
    assert!(bindings[0].source.file.ends_with("registry_provider.rs"));

    // Class 与 Factory 归一化为实例候选，Bound 只进入投影规则表。
    assert_eq!(registry.all_providers().count(), 2);
    assert_eq!(registry.all_bindings().count(), 1);
}
