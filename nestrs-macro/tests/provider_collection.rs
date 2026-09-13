//! 注册链路验证：三个属性宏生成的 `Provider` 都经 linkme 收进同一切片，
//! 并且各自保留自己的声明形态，不被提前归一化。

use nestrs_core::{
    __private::{FactoryInvoker, Provider, REFLECTED_PROVIDERS},
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
fn linkme_collects_class_factory_and_bound_registrations() {
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

    let binding = providers
        .iter()
        .find_map(|provider| match provider {
            Provider::Bound(binding) => Some(binding),
            _ => None,
        })
        .expect("bind should register a trait binding");
    assert_eq!(binding.trait_type, ServiceType::create::<dyn Greeter>());
    assert_eq!(
        binding.concrete_type,
        ServiceType::create::<GreeterService>()
    );
    assert!(binding.source.file.ends_with("provider_collection.rs"));

    assert_eq!(providers.len(), 3);
}
