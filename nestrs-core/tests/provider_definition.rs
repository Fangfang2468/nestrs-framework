use std::marker::PhantomData;

use nestrs_core::{
    __private::{
        ActivationError, ClosedProviderCallback, ConstructionContext, ErasedService, InjectionSpec,
        InjectionTarget, InputPosition, Provider, ProviderCommon, ProviderDefinition,
        provider_definition,
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
        Provider::Class {
            provide: ServiceIdentifier::from(ServiceType::create::<Self>()),
            common: ProviderCommon {
                lifetime: Lifetime::Singleton,
                primary: false,
                source: ServiceSource::new("provider_definition.rs", 1, 1),
                cleanup: None,
            },
            dependencies: vec![],
            constructor: construct_repository::<T>,
        }
    }
}

#[test]
fn closed_provider_callback_is_specialized_for_the_closed_dependency_type() {
    let callback: ClosedProviderCallback = provider_definition::<Repository<Entity>>;
    let injection = InjectionSpec {
        declaration_position: 0,
        input_position: InputPosition(0),
        label: Some("repository"),
        token: ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>()),
        optional: false,
        target: InjectionTarget::Concrete,
        prepare_input: None,
        closed_provider: Some(callback),
    };
    let definition = injection
        .closed_provider
        .expect("closed generic injection should retain its provider callback");

    let Provider::Class {
        provide,
        common,
        dependencies,
        constructor,
    } = definition()
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
