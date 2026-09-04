use nestrs_core::{
    __private::{ConstructionContext, FieldInjectionTarget, REFLECT_METADATA_INJECTABLE},
    lifetime::Lifetime,
    registration::{
        service_identifier::ServiceIdentifier, service_key::ServiceKey, service_type::ServiceType,
    },
};
use nestrs_macro::{injectable, primary};

struct Database;

trait Audit: Send + Sync {}

static STATIC_LABEL: &str = "from-static";

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

#[test]
fn injectable_collects_provider_and_field_metadata() {
    let components: Vec<_> = REFLECT_METADATA_INJECTABLE
        .iter()
        .map(|metadata| metadata())
        .collect();
    let controller = components
        .iter()
        .find(|component| {
            component.service_identifier
                == ServiceIdentifier::new(
                    Some(ServiceKey::Named("controller")),
                    ServiceType::create::<Controller>(),
                )
        })
        .expect("injectable macro should collect Controller metadata");

    assert_eq!(controller.lifetime, Lifetime::Scoped);
    assert!(!controller.primary);
    assert!(controller.source.file.ends_with("injectable_metadata.rs"));
    assert_eq!(controller.field_injections.len(), 2);

    let database = &controller.field_injections[0];
    assert_eq!(database.field_index, 0);
    assert_eq!(database.field_name, Some("database"));
    assert_eq!(database.dependency_position, 0);
    assert_eq!(
        database.service_identifier,
        ServiceIdentifier::from(ServiceType::create::<Database>())
    );
    assert_eq!(database.target, FieldInjectionTarget::Concrete);
    assert!(database.prepare_input.is_some());
    assert!(!database.optional);

    let audit = &controller.field_injections[1];
    assert_eq!(audit.field_index, 3);
    assert_eq!(audit.field_name, Some("audit"));
    assert_eq!(audit.dependency_position, 1);
    assert_eq!(
        audit.service_identifier,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("audit")),
            ServiceType::create::<dyn Audit>(),
        )
    );
    assert_eq!(audit.target, FieldInjectionTarget::TraitObject);
    assert!(audit.prepare_input.is_some());
    assert!(audit.optional);

    let tuple = components
        .iter()
        .find(|component| {
            component.service_identifier.service_type == ServiceType::create::<TupleConsumer>()
        })
        .expect("injectable macro should collect tuple metadata");
    assert_eq!(tuple.field_injections[0].field_index, 0);
    assert_eq!(tuple.field_injections[0].field_name, None);
    assert_eq!(tuple.field_injections[0].dependency_position, 0);
    assert_eq!(
        tuple.field_injections[0].service_identifier,
        ServiceIdentifier::new(
            Some(ServiceKey::Indexed(7)),
            ServiceType::create::<Database>(),
        )
    );

    let values = components
        .iter()
        .find(|component| {
            component.service_identifier.service_type == ServiceType::create::<FieldValues>()
        })
        .expect("injectable macro should collect FieldValues metadata");
    let erased_values = (values.constructor)(ConstructionContext::new())
        .expect("value-only constructor should not need dependency inputs");
    let values = match erased_values.downcast::<FieldValues>() {
        Ok(values) => values,
        Err(_) => panic!("generated constructor should return FieldValues"),
    };

    assert_eq!(values.owned_literal, "123");
    assert_eq!(values.static_label, "from-static");
    assert_eq!(values.expression, 42);
    assert!(values.defaults.is_empty());

    let primary_before_injectable = components
        .iter()
        .find(|component| {
            component.service_identifier.service_type
                == ServiceType::create::<PrimaryBeforeInjectable>()
        })
        .expect("primary-before-injectable should collect component metadata");
    assert!(primary_before_injectable.primary);

    let injectable_before_primary = components
        .iter()
        .find(|component| {
            component.service_identifier.service_type
                == ServiceType::create::<InjectableBeforePrimary>()
        })
        .expect("injectable-before-primary should collect component metadata");
    assert!(injectable_before_primary.primary);
}
