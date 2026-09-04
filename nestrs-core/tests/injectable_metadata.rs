use nestrs_core::{
    __private::{
        ActivationError, ConstructionContext, ErasedService, FieldInjection, FieldInjectionTarget,
        REFLECT_METADATA_INJECTABLE, StructComponent,
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
    ::nestrs_core::__private::REFLECT_METADATA_INJECTABLE
)]
#[linkme(crate = ::nestrs_core::__private::linkme)]
fn component_metadata() -> StructComponent {
    StructComponent {
        service_identifier: ServiceIdentifier::new(
            Some(ServiceKey::Named("controller")),
            ServiceType::create::<Component>(),
        ),
        lifetime: Lifetime::Scoped,
        field_injections: vec![
            FieldInjection {
                field_index: 0,
                field_name: Some("database"),
                dependency_position: 0,
                service_identifier: ServiceIdentifier::from(ServiceType::create::<Database>()),
                target: FieldInjectionTarget::Concrete,
                component_definition: None,
                prepare_input: None,
                optional: false,
            },
            FieldInjection {
                field_index: 2,
                field_name: None,
                dependency_position: 1,
                service_identifier: ServiceIdentifier::new(
                    Some(ServiceKey::Indexed(7)),
                    ServiceType::create::<dyn Audit>(),
                ),
                target: FieldInjectionTarget::TraitObject,
                component_definition: None,
                prepare_input: None,
                optional: true,
            },
        ],
        constructor: construct_component,
        primary: true,
        source: ServiceSource::new("injectable_metadata.rs", 30, 1),
    }
}

#[test]
fn struct_component_keeps_provider_identity_and_field_input_layout() {
    let components: Vec<_> = REFLECT_METADATA_INJECTABLE
        .iter()
        .map(|metadata| metadata())
        .collect();
    let component = components
        .iter()
        .find(|component| {
            component.service_identifier.service_type == ServiceType::create::<Component>()
        })
        .expect("test component metadata should be collected through linkme");

    assert_eq!(
        component.service_identifier,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("controller")),
            ServiceType::create::<Component>(),
        )
    );
    assert_eq!(component.lifetime, Lifetime::Scoped);
    assert!(component.primary);
    assert_eq!(component.field_injections.len(), 2);

    let required = &component.field_injections[0];
    assert_eq!(required.field_index, 0);
    assert_eq!(required.field_name, Some("database"));
    assert_eq!(required.dependency_position, 0);
    assert!(!required.optional);

    let optional_tuple = &component.field_injections[1];
    assert_eq!(optional_tuple.field_index, 2);
    assert_eq!(optional_tuple.field_name, None);
    assert_eq!(optional_tuple.dependency_position, 1);
    assert!(optional_tuple.optional);
    assert_eq!(
        optional_tuple.service_identifier,
        ServiceIdentifier::new(
            Some(ServiceKey::Indexed(7)),
            ServiceType::create::<dyn Audit>(),
        )
    );
}
