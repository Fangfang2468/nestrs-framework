use std::marker::PhantomData;

use nestrs_core::{
    __private::{
        ActivationError, ClassProvider, ClosedProviderCallback, ConstructionContext,
        DependencyRequest, Delivery, ErasedService, InputPosition, Provider, ProviderCommon,
        ProviderDefinition, ProviderSource, prepare_required, provider_definition,
    },
    lifetime::Lifetime,
    registration::{
        service_identifier::ServiceIdentifier, service_source::ServiceSource,
        service_type::ServiceType,
    },
};

struct Entity;
struct Repository<T>(PhantomData<T>);

fn construct_repository<T>(_context: ConstructionContext) -> Result<ErasedService, ActivationError>
where
    T: Send + Sync + 'static,
{
    Ok(ErasedService::new(Repository::<T>(PhantomData)))
}

impl<T> ProviderDefinition for Repository<T>
where
    T: Send + Sync + 'static,
{
    fn provider() -> Provider {
        Provider::Class(ClassProvider {
            provide: ServiceIdentifier::from(ServiceType::create::<Self>()),
            common: ProviderCommon {
                lifetime: Lifetime::Singleton,
                primary: false,
                source: ServiceSource::new("provider_definition.rs", 1, 1),
                cleanup: None,
            },
            dependencies: vec![],
            constructor: construct_repository::<T>,
        })
    }
}

#[test]
fn closed_provider_callback_is_specialized_for_the_closed_dependency_type() {
    let callback: ClosedProviderCallback = provider_definition::<Repository<Entity>>;
    let injection = DependencyRequest {
        declaration_position: 0,
        input_position: InputPosition(0),
        label: Some("repository"),
        token: ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>()),
        optional: false,
        delivery: Delivery::Direct(prepare_required::<Repository<Entity>>),
        provider_source: ProviderSource::Materialize(callback),
    };
    let definition = match injection.provider_source {
        ProviderSource::Materialize(definition) => definition,
        ProviderSource::Registered => {
            panic!("closed generic injection should retain its provider callback")
        }
    };

    let Provider::Class(ClassProvider {
        provide,
        common,
        dependencies,
        constructor,
    }) = definition()
    else {
        panic!("closed generic callback should produce a class provider")
    };

    assert_eq!(
        provide,
        ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>())
    );
    assert_eq!(common.lifetime, Lifetime::Singleton);
    assert!(!common.primary);
    assert!(dependencies.is_empty());

    let erased = constructor(ConstructionContext::new())
        .expect("closed generic provider should construct its concrete type");
    assert!(matches!(erased.downcast::<Repository<Entity>>(), Ok(_)));
}
