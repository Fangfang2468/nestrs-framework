use std::sync::atomic::{AtomicUsize, Ordering};

use nestrs_core::registration::{
    service_collection::{InstantiationError, ServiceCollection},
    service_identifier::ServiceIdentifier,
    service_type::ServiceType,
};
use nestrs_macro::{bind, injectable, primary};

static REPOSITORY_CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct Repository {
    #[value(REPOSITORY_CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst))]
    construction_index: usize,
}

#[injectable]
struct ConcreteConsumer {
    #[inject]
    first: Repository,
    #[inject]
    second: Repository,
}

struct OptionalMissing;

#[injectable]
struct OptionalConsumer {
    #[inject]
    dependency: Option<OptionalMissing>,
}

struct RequiredMissing;

#[injectable]
struct RequiredConsumer {
    #[inject]
    dependency: RequiredMissing,
}

trait UnprojectedPort: Send + Sync {}

#[injectable]
struct TraitConsumer {
    #[inject]
    dependency: dyn UnprojectedPort,
}

trait GreetingPort: Send + Sync {
    fn greet(&self) -> &'static str;
}

#[injectable]
struct GreetingService;

#[bind]
impl GreetingPort for GreetingService {
    fn greet(&self) -> &'static str {
        "hello from bound service"
    }
}

#[injectable]
struct BoundTraitConsumer {
    #[inject]
    dependency: dyn GreetingPort,
}

trait OptionalUnboundPort: Send + Sync {}

#[injectable]
struct OptionalTraitConsumer {
    #[inject]
    dependency: Option<dyn OptionalUnboundPort>,
}

trait PrimaryPort: Send + Sync {
    fn selected_name(&self) -> &'static str;
}

#[injectable]
struct SecondaryPrimaryService;

#[bind]
impl PrimaryPort for SecondaryPrimaryService {
    fn selected_name(&self) -> &'static str {
        "secondary"
    }
}

#[primary]
#[injectable]
struct PreferredPrimaryService;

#[bind]
impl PrimaryPort for PreferredPrimaryService {
    fn selected_name(&self) -> &'static str {
        "primary"
    }
}

#[injectable]
struct PrimaryTraitConsumer {
    #[inject]
    dependency: dyn PrimaryPort,
}

trait AmbiguousPort: Send + Sync {}

#[injectable]
struct FirstAmbiguousService;

#[bind]
impl AmbiguousPort for FirstAmbiguousService {}

#[injectable]
struct SecondAmbiguousService;

#[bind]
impl AmbiguousPort for SecondAmbiguousService {}

#[injectable]
struct AmbiguousTraitConsumer {
    #[inject]
    dependency: dyn AmbiguousPort,
}

trait KeyedPort: Send + Sync {
    fn key_name(&self) -> &'static str;
}

#[injectable(key = "blue")]
struct BlueKeyedService;

#[bind]
impl KeyedPort for BlueKeyedService {
    fn key_name(&self) -> &'static str {
        "blue"
    }
}

#[injectable(key = "red")]
struct RedKeyedService;

#[bind]
impl KeyedPort for RedKeyedService {
    fn key_name(&self) -> &'static str {
        "red"
    }
}

#[injectable]
struct KeyedTraitConsumer {
    #[inject(key = "red")]
    dependency: dyn KeyedPort,
}

#[injectable]
struct RecursiveLeft {
    #[inject]
    right: RecursiveRight,
}

#[injectable]
struct RecursiveRight {
    #[inject]
    left: RecursiveLeft,
}

#[test]
fn instantiates_concrete_dependencies_once_and_binds_both_fields_to_the_arena_address() {
    REPOSITORY_CONSTRUCTIONS.store(0, Ordering::SeqCst);

    let arena = ServiceCollection::new()
        .instantiate::<ConcreteConsumer>()
        .expect("concrete dependencies should be instantiated before their consumer");
    let consumer = arena
        .get::<ConcreteConsumer>()
        .expect("consumer should be committed after its dependencies");
    let repository = arena
        .get::<Repository>()
        .expect("shared dependency should be committed exactly once");

    assert_eq!(REPOSITORY_CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(repository.construction_index, 0);
    assert!(std::ptr::eq(&*consumer.first, &*consumer.second,));
    assert!(std::ptr::eq(&*consumer.first, repository));
}

#[test]
fn delivers_none_when_an_optional_concrete_dependency_has_no_component() {
    let arena = ServiceCollection::new()
        .instantiate::<OptionalConsumer>()
        .expect("missing optional dependency should not prevent construction");
    let consumer = arena
        .get::<OptionalConsumer>()
        .expect("optional consumer should be committed");

    assert!(consumer.dependency.is_none());
}

#[test]
fn reports_a_missing_required_concrete_dependency_before_calling_the_constructor() {
    let error = match ServiceCollection::new().instantiate::<RequiredConsumer>() {
        Ok(_) => panic!("missing required dependency must be reported"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        InstantiationError::MissingDependency {
            provider,
            dependency,
        } if provider == ServiceIdentifier::from(ServiceType::create::<RequiredConsumer>())
            && dependency == ServiceIdentifier::from(ServiceType::create::<RequiredMissing>())
    ));
}

#[test]
fn reports_a_missing_required_trait_dependency_when_no_bind_exists() {
    let error = match ServiceCollection::new().instantiate::<TraitConsumer>() {
        Ok(_) => panic!("unbound required trait dependency must be reported"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        InstantiationError::MissingDependency {
            provider,
            dependency,
        } if provider == ServiceIdentifier::from(ServiceType::create::<TraitConsumer>())
            && dependency == ServiceIdentifier::from(ServiceType::create::<dyn UnprojectedPort>())
    ));
}

#[test]
fn projects_a_bound_concrete_service_to_the_requested_dyn_trait() {
    let arena = ServiceCollection::new()
        .instantiate::<BoundTraitConsumer>()
        .expect("a matching #[bind] should prepare a trait-object input");
    let consumer = arena
        .get::<BoundTraitConsumer>()
        .expect("bound trait consumer should be committed");

    assert_eq!(consumer.dependency.greet(), "hello from bound service");
}

#[test]
fn delivers_none_when_an_optional_trait_dependency_has_no_bind() {
    let arena = ServiceCollection::new()
        .instantiate::<OptionalTraitConsumer>()
        .expect("an unbound optional trait dependency should not prevent construction");
    let consumer = arena
        .get::<OptionalTraitConsumer>()
        .expect("optional trait consumer should be committed");

    assert!(consumer.dependency.is_none());
}

#[test]
fn uses_the_unique_primary_bound_service_when_multiple_bindings_match() {
    let arena = ServiceCollection::new()
        .instantiate::<PrimaryTraitConsumer>()
        .expect("a unique primary binding should resolve an otherwise ambiguous trait request");
    let consumer = arena
        .get::<PrimaryTraitConsumer>()
        .expect("primary trait consumer should be committed");

    assert_eq!(consumer.dependency.selected_name(), "primary");
}

#[test]
fn reports_ambiguous_trait_bindings_without_a_unique_primary() {
    let error = match ServiceCollection::new().instantiate::<AmbiguousTraitConsumer>() {
        Ok(_) => panic!("multiple equally eligible bindings must be rejected"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        InstantiationError::AmbiguousTraitBinding {
            provider,
            field_name: Some("dependency"),
            candidates,
        } if provider == ServiceIdentifier::from(ServiceType::create::<AmbiguousTraitConsumer>())
            && candidates.len() == 2
            && candidates.contains(&ServiceIdentifier::from(ServiceType::create::<FirstAmbiguousService>()))
            && candidates.contains(&ServiceIdentifier::from(ServiceType::create::<SecondAmbiguousService>()))
    ));
}

#[test]
fn filters_bound_services_by_the_trait_field_key() {
    let arena = ServiceCollection::new()
        .instantiate::<KeyedTraitConsumer>()
        .expect("the trait field key should select the matching keyed concrete service");
    let consumer = arena
        .get::<KeyedTraitConsumer>()
        .expect("keyed trait consumer should be committed");

    assert_eq!(consumer.dependency.key_name(), "red");
}

#[test]
fn prevents_activation_reentry_without_building_or_validating_a_dependency_graph() {
    let error = match ServiceCollection::new().instantiate::<RecursiveLeft>() {
        Ok(_) => panic!("recursive construction must stop before exhausting the stack"),
        Err(error) => error,
    };

    assert!(matches!(
        error,
        InstantiationError::ReentrantInstantiation { identifier }
            if identifier == ServiceIdentifier::from(ServiceType::create::<RecursiveLeft>())
    ));
}
