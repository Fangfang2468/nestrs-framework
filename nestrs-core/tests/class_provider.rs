use nestrs_core::__private::{
    ActivationError, ClassProvider, ConstructionContext, Delivery, DependencyRequest,
    ErasedService, InputPosition, Lifetime, Provider, ProviderCommon, ProviderSource,
    REFLECTED_PROVIDERS, ServiceIdentifier, ServiceKey, ServiceSource, ServiceType,
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
    Provider::Class(ClassProvider {
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
            DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                label: Some("database"),
                token: ServiceIdentifier::from(ServiceType::create::<Database>()),
                optional: false,
                delivery: Delivery::Direct(nestrs_core::__private::prepare_required::<Database>),
                provider_source: ProviderSource::Registered,
            },
            DependencyRequest {
                declaration_position: 2,
                input_position: InputPosition(1),
                label: None,
                token: ServiceIdentifier::new(
                    Some(ServiceKey::Indexed(7)),
                    ServiceType::create::<dyn Audit>(),
                ),
                optional: true,
                delivery: Delivery::RequiresBindingOrAbsent(
                    nestrs_core::__private::prepare_optional_absent::<dyn Audit>,
                ),
                provider_source: ProviderSource::Registered,
            },
        ],
        constructor: construct_component,
    })
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
                Provider::Class(ClassProvider { provide, .. })
                    if provide.service_type == ServiceType::create::<Component>()
            )
        })
        .expect("test class provider should be collected through linkme");

    let Provider::Class(ClassProvider {
        provide,
        common,
        dependencies,
        ..
    }) = provider
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
    assert!(matches!(required.delivery, Delivery::Direct(_)));
    assert!(matches!(
        required.provider_source,
        ProviderSource::Registered
    ));

    let optional_tuple = &dependencies[1];
    assert_eq!(optional_tuple.declaration_position, 2);
    assert_eq!(optional_tuple.input_position, InputPosition(1));
    assert_eq!(optional_tuple.label, None);
    assert!(optional_tuple.optional);
    assert!(matches!(
        optional_tuple.delivery,
        Delivery::RequiresBindingOrAbsent(_)
    ));
    assert_eq!(
        optional_tuple.token,
        ServiceIdentifier::new(
            Some(ServiceKey::Indexed(7)),
            ServiceType::create::<dyn Audit>(),
        )
    );
}
