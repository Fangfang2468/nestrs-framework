use nestrs_core::{
    __private::{
        ActivationError, ConstructionContext, ErasedService, InjectionSpec, InjectionTarget,
        InputPosition, Provider, ProviderCommon, REFLECTED_PROVIDERS,
    },
    lifetime::Lifetime,
    registration::{
        service_identifier::ServiceIdentifier, service_key::ServiceKey,
        service_source::ServiceSource, service_type::ServiceType,
    },
};

struct Component;
struct Database;
trait Audit: Send + Sync {}

fn construct_component(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
    Ok(ErasedService::new(Component))
}

#[::nestrs_core::__private::linkme::distributed_slice(
    ::nestrs_core::__private::REFLECTED_PROVIDERS
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn component_provider() -> Provider {
    Provider::Class {
        provide: ServiceIdentifier::new(
            Some(ServiceKey::Named("controller")),
            ServiceType::create::<Component>(),
        ),
        common: ProviderCommon {
            lifetime: Lifetime::Scoped,
            primary: true,
            source: ServiceSource::new("class_provider.rs", 30, 1),
            cleanup: None,
        },
        dependencies: vec![
            InjectionSpec {
                declaration_position: 0,
                input_position: InputPosition(0),
                label: Some("database"),
                token: ServiceIdentifier::from(ServiceType::create::<Database>()),
                optional: false,
                target: InjectionTarget::Concrete,
                prepare_input: None,
                closed_provider: None,
            },
            InjectionSpec {
                declaration_position: 2,
                input_position: InputPosition(1),
                label: None,
                token: ServiceIdentifier::new(
                    Some(ServiceKey::Indexed(7)),
                    ServiceType::create::<dyn Audit>(),
                ),
                optional: true,
                target: InjectionTarget::TraitObject,
                prepare_input: None,
                closed_provider: None,
            },
        ],
        constructor: construct_component,
    }
}

#[test]
fn class_provider_keeps_provider_identity_and_dependency_input_layout() {
    let providers: Vec<_> = REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .collect();
    let provider = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if provide.service_type == ServiceType::create::<Component>()
            )
        })
        .expect("test class provider should be collected through linkme");

    let Provider::Class {
        provide,
        common,
        dependencies,
        ..
    } = provider
    else {
        panic!("selected registration should be a class provider")
    };

    assert_eq!(
        *provide,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("controller")),
            ServiceType::create::<Component>(),
        )
    );
    assert_eq!(common.lifetime, Lifetime::Scoped);
    assert!(common.primary);
    assert!(common.cleanup.is_none());
    assert_eq!(dependencies.len(), 2);

    let required = &dependencies[0];
    assert_eq!(required.declaration_position, 0);
    assert_eq!(required.input_position, InputPosition(0));
    assert_eq!(required.label, Some("database"));
    assert!(!required.optional);
    assert_eq!(required.target, InjectionTarget::Concrete);

    let optional_tuple = &dependencies[1];
    assert_eq!(optional_tuple.declaration_position, 2);
    assert_eq!(optional_tuple.input_position, InputPosition(1));
    assert_eq!(optional_tuple.label, None);
    assert!(optional_tuple.optional);
    assert_eq!(optional_tuple.target, InjectionTarget::TraitObject);
    assert_eq!(
        optional_tuple.token,
        ServiceIdentifier::new(
            Some(ServiceKey::Indexed(7)),
            ServiceType::create::<dyn Audit>(),
        )
    );
}
