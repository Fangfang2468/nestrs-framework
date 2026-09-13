//! 注册链路验证：实例 provider 与 trait 绑定分别收进各自的 linkme 切片。
//!
//! `#[injectable]` / `#[factory]` 产出实例 provider，进入 `REFLECTED_PROVIDERS`；
//! `#[bind]` 产出 trait 绑定，进入 `REFLECTED_BINDINGS`——绑定不是 provider。

use nestrs_core::{
    __private::{FactoryInvoker, Provider, REFLECTED_BINDINGS, REFLECTED_PROVIDERS, TraitBinding},
    registration::{
        service_identifier::ServiceIdentifier, service_key::ServiceKey,
        service_type::ServiceType,
    },
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
fn linkme_collects_providers_and_trait_bindings_separately() {
    let providers: Vec<Provider> = REFLECTED_PROVIDERS.iter().map(|entry| entry()).collect();

    let repository = ServiceIdentifier::from(ServiceType::create::<Repository>());
    let class = providers
        .iter()
        .find_map(|provider| match provider {
            Provider::Class(class) if class.provide == repository => Some(class),
            _ => None,
        })
        .expect("injectable should register a class provider for Repository");
    assert!(class.dependencies.is_empty());
    assert!(class.common.cleanup.is_none());

    let greeter = ServiceIdentifier::new(
        Some(ServiceKey::Named("greeting")),
        ServiceType::create::<GreeterService>(),
    );
    let factory = providers
        .iter()
        .find_map(|provider| match provider {
            Provider::Factory(factory) if factory.provide == greeter => Some(factory),
            _ => None,
        })
        .expect("factory should register a factory provider for the keyed token");
    assert!(matches!(factory.invoker, FactoryInvoker::Sync(_)));

    // provider 切片里只有实例生产者，绑定不在此处。
    assert_eq!(providers.len(), 2);

    let bindings: Vec<TraitBinding> = REFLECTED_BINDINGS.iter().map(|entry| entry()).collect();
    assert_eq!(bindings.len(), 1);
    let binding = &bindings[0];
    assert_eq!(binding.trait_type, ServiceType::create::<dyn Greeter>());
    assert_eq!(
        binding.concrete_type,
        ServiceType::create::<GreeterService>()
    );
    assert!(binding.source.file.ends_with("registration_collection.rs"));
}
