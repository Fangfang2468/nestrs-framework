use std::marker::PhantomData;

use nestrs_core::{
    __private::{ComponentDefinition, ConstructionContext, REFLECT_METADATA_INJECTABLE},
    registration::{
        service_collection::ServiceCollection, service_identifier::ServiceIdentifier,
        service_type::ServiceType,
    },
};
use nestrs_macro::injectable;

struct User;
struct Entity;

/// 泛型 injectable 本身不应向 linkme 写入一个开放类型的 provider；具体类型的
/// metadata 由 `ComponentDefinition` 在依赖使用处按需物化。
#[injectable]
struct Repository<T> {
    #[value("generic-repository")]
    label: String,
    marker: PhantomData<T>,
}

#[injectable]
struct UserService {
    #[inject]
    repository: Repository<User>,
}

#[test]
fn generic_injectable_materializes_concrete_component_definitions() {
    let direct = <Repository<Entity> as ComponentDefinition>::component();

    assert_eq!(
        direct.service_identifier,
        ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>())
    );
    assert!(direct.field_injections.is_empty());

    let erased_direct_repository = (direct.constructor)(ConstructionContext::new())
        .expect("generic Repository<Entity> constructor should not need dependencies");
    let direct_repository = match erased_direct_repository.downcast::<Repository<Entity>>() {
        Ok(repository) => repository,
        Err(_) => panic!("generic component definition should retain its concrete type"),
    };
    assert_eq!(direct_repository.label, "generic-repository");
}

#[test]
fn injected_generic_repository_exposes_a_concrete_component_callback() {
    let components: Vec<_> = REFLECT_METADATA_INJECTABLE
        .iter()
        .map(|metadata| metadata())
        .collect();
    let user_service = components
        .iter()
        .find(|component| {
            component.service_identifier.service_type == ServiceType::create::<UserService>()
        })
        .expect("non-generic UserService should be collected by linkme");

    // `Repository<T>` is an open generic definition, so it must not register an arbitrary
    // `Repository<Entity>` or `Repository<User>` provider eagerly.
    assert!(!components.iter().any(|component| {
        component.service_identifier.service_type == ServiceType::create::<Repository<Entity>>()
            || component.service_identifier.service_type
                == ServiceType::create::<Repository<User>>()
    }));

    let dependency = user_service
        .field_injections
        .first()
        .expect("UserService should describe its Repository<User> dependency");
    assert_eq!(dependency.dependency_position, 0);
    assert_eq!(
        dependency.service_identifier,
        ServiceIdentifier::from(ServiceType::create::<Repository<User>>())
    );

    let repository_definition = dependency
        .component_definition
        .expect("generic injection should carry its concrete component definition");
    let repository_component = repository_definition();
    assert_eq!(
        repository_component.service_identifier,
        ServiceIdentifier::from(ServiceType::create::<Repository<User>>())
    );

    let erased_repository = (repository_component.constructor)(ConstructionContext::new())
        .expect("Repository<User> component callback should construct the concrete service");
    let repository = match erased_repository.downcast::<Repository<User>>() {
        Ok(repository) => repository,
        Err(_) => panic!("generic dependency callback should retain Repository<User>"),
    };
    assert_eq!(repository.label, "generic-repository");

    let collection = ServiceCollection::new();
    let arena = collection
        .instantiate::<UserService>()
        .expect("generic dependency should be materialized and injected into UserService");
    let user_service = arena
        .get::<UserService>()
        .expect("UserService should be committed to the arena");
    let repository = arena
        .get::<Repository<User>>()
        .expect("closed generic Repository<User> should be committed once");

    assert_eq!(user_service.repository.label, "generic-repository");
    assert_eq!(
        &*user_service.repository as *const Repository<User>, repository as *const Repository<User>,
        "Inject must point at the Arena-owned generic dependency"
    );
}
