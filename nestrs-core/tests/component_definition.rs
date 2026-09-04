use std::marker::PhantomData;

use nestrs_core::{
    __private::{
        ActivationError, ComponentDefinition, ComponentDefinitionCallback, ConstructionContext,
        ErasedService, FieldInjection, FieldInjectionTarget, StructComponent, component_definition,
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

impl<T> ComponentDefinition for Repository<T>
where
    T: Send + Sync + 'static,
{
    fn component() -> StructComponent {
        StructComponent {
            service_identifier: ServiceIdentifier::from(ServiceType::create::<Self>()),
            lifetime: Lifetime::Singleton,
            field_injections: vec![],
            constructor: construct_repository::<T>,
            primary: false,
            source: ServiceSource::new("component_definition.rs", 1, 1),
        }
    }
}

#[test]
fn component_definition_callback_is_specialized_for_the_closed_dependency_type() {
    let callback: ComponentDefinitionCallback = component_definition::<Repository<Entity>>;
    let injection = FieldInjection {
        field_index: 0,
        field_name: Some("repository"),
        dependency_position: 0,
        service_identifier: ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>()),
        target: FieldInjectionTarget::Concrete,
        component_definition: Some(callback),
        prepare_input: None,
        optional: false,
    };
    let definition = injection
        .component_definition
        .expect("closed generic injection should retain its definition callback");
    let component = definition();

    assert_eq!(
        component.service_identifier,
        ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>())
    );
    assert_eq!(component.lifetime, Lifetime::Singleton);
    assert!(component.field_injections.is_empty());
}
