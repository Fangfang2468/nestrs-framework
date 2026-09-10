use nestrs_core::{
    __private::{ConstructionContext, InjectionTarget, Provider, REFLECTED_PROVIDERS},
    lifetime::Lifetime,
    registration::{
        service_identifier::ServiceIdentifier, service_key::ServiceKey, service_type::ServiceType,
    },
};
use nestrs_macro::{injectable, primary};

struct Database;

trait Audit: Send + Sync {}

static STATIC_LABEL: &str = "from-static";

async fn cleanup_controller() {}

// 与旧版宏的第一个临时字段标识符同名，用于回归调用点路径不会被内部实现遮蔽。
#[allow(non_upper_case_globals)]
const __nestrs_field_0: usize = 41;

#[injectable(lifetime = Scoped, key = "controller")]
struct Controller {
    #[inject]
    database: Database,
    #[value(3)]
    retries: usize,
    #[value("controller")]
    name: String,
    #[inject(key = "audit")]
    audit: Option<dyn Audit>,
    enabled: bool,
}

#[injectable]
struct TupleConsumer(#[inject(7)] Database);

#[injectable]
struct FieldValues {
    #[value("123".to_owned())]
    owned_literal: String,
    #[value(STATIC_LABEL)]
    static_label: String,
    #[value(__nestrs_field_0 + 1)]
    expression: usize,
    defaults: Vec<String>,
}

// `primary` 先展开时，它会在尚未展开的 `injectable` 后面追加内部 marker。
#[primary]
#[injectable]
struct PrimaryBeforeInjectable;

// `injectable` 先展开时，必须直接消费下方的 `primary`，使 primary 宏不再执行。
#[injectable]
#[primary()]
struct InjectableBeforePrimary;

#[injectable(cleanup = "cleanup_controller")]
struct CleanupController;

#[test]
fn injectable_collects_class_providers_and_dependency_specs() {
    let providers: Vec<_> = REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .collect();
    let controller = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if *provide
                        == ServiceIdentifier::new(
                            Some(ServiceKey::Named("controller")),
                            ServiceType::create::<Controller>(),
                        )
            )
        })
        .expect("injectable macro should collect Controller provider");
    let Provider::Class {
        common,
        dependencies,
        ..
    } = controller
    else {
        panic!("Controller should be a class provider");
    };

    assert_eq!(common.lifetime, Lifetime::Scoped);
    assert!(!common.primary);
    assert!(common.source.file.ends_with("injectable_provider.rs"));
    assert!(common.cleanup.is_none());
    assert_eq!(dependencies.len(), 2);

    let database = &dependencies[0];
    assert_eq!(database.declaration_position, 0);
    assert_eq!(database.label, Some("database"));
    assert_eq!(database.input_position.0, 0);
    assert_eq!(
        database.token,
        ServiceIdentifier::from(ServiceType::create::<Database>())
    );
    assert_eq!(database.target, InjectionTarget::Concrete);
    assert!(database.prepare_input.is_some());
    assert!(database.closed_provider.is_none());
    assert!(!database.optional);

    let audit = &dependencies[1];
    assert_eq!(audit.declaration_position, 3);
    assert_eq!(audit.label, Some("audit"));
    assert_eq!(audit.input_position.0, 1);
    assert_eq!(
        audit.token,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("audit")),
            ServiceType::create::<dyn Audit>(),
        )
    );
    assert_eq!(audit.target, InjectionTarget::TraitObject);
    assert!(audit.prepare_input.is_some());
    assert!(audit.closed_provider.is_none());
    assert!(audit.optional);

    let tuple = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if provide.service_type == ServiceType::create::<TupleConsumer>()
            )
        })
        .expect("injectable macro should collect tuple provider");
    let Provider::Class { dependencies, .. } = tuple else {
        panic!("TupleConsumer should be a class provider");
    };
    assert_eq!(dependencies[0].declaration_position, 0);
    assert_eq!(dependencies[0].label, None);
    assert_eq!(dependencies[0].input_position.0, 0);
    assert_eq!(
        dependencies[0].token,
        ServiceIdentifier::new(
            Some(ServiceKey::Indexed(7)),
            ServiceType::create::<Database>(),
        )
    );

    let values = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if provide.service_type == ServiceType::create::<FieldValues>()
            )
        })
        .expect("injectable macro should collect FieldValues provider");
    let Provider::Class { constructor, .. } = values else {
        panic!("FieldValues should be a class provider");
    };
    let erased_values = constructor(ConstructionContext::new())
        .expect("value-only constructor should not need dependency inputs");
    let values = match erased_values.downcast::<FieldValues>() {
        Ok(values) => values,
        Err(_) => panic!("generated constructor should return FieldValues"),
    };

    assert_eq!(values.owned_literal, "123");
    assert_eq!(values.static_label, "from-static");
    assert_eq!(values.expression, 42);
    assert!(values.defaults.is_empty());

    let primary_before_injectable = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if provide.service_type == ServiceType::create::<PrimaryBeforeInjectable>()
            )
        })
        .expect("primary-before-injectable should collect a class provider");
    let Provider::Class { common, .. } = primary_before_injectable else {
        panic!("primary-before-injectable should be a class provider");
    };
    assert!(common.primary);

    let injectable_before_primary = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if provide.service_type == ServiceType::create::<InjectableBeforePrimary>()
            )
        })
        .expect("injectable-before-primary should collect a class provider");
    let Provider::Class { common, .. } = injectable_before_primary else {
        panic!("injectable-before-primary should be a class provider");
    };
    assert!(common.primary);

    let cleanup_controller = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class { provide, .. }
                    if provide.service_type == ServiceType::create::<CleanupController>()
            )
        })
        .expect("injectable cleanup should be retained by its provider");
    let Provider::Class { common, .. } = cleanup_controller else {
        panic!("CleanupController should be a class provider");
    };
    let cleanup = common
        .cleanup
        .expect("injectable cleanup should be adapted to CleanupHook");
    drop(cleanup());
}
