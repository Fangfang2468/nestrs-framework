use crate::activation::{ConstructionInputs, InputSlot};
use crate::graph::Constructor;
use crate::lifetime::ServiceLifetime;
use crate::service::{ServiceIdentifier, ServiceKey, ServiceType};
use nestrs::{injectable, primary};

struct Database;

// 完整入口现在在编译期间验证全部 provider。声明 ABI 测试也提供真实依赖，避免把
// “测试只读取元数据”当成允许生产图缺少必选依赖的特殊通道。
#[nestrs::factory]
fn database() -> Database {
    Database
}

#[nestrs::factory(key = 7)]
fn indexed_database() -> Database {
    Database
}

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
    #[inject("audit")]
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

#[injectable(lifetime = Transient)]
struct TransientController;

// primary 已在编译期消费，不应为断言重放到运行期。分别加入实际竞争实现，
// 用已选接口路由验证两种属性顺序仍选择原来的 primary 服务。
trait BeforePort: Send + Sync {}
trait AfterPort: Send + Sync {}
#[injectable]
struct Secondary;
impl BeforePort for PrimaryBeforeInjectable {}
impl BeforePort for Secondary {}
impl AfterPort for InjectableBeforePrimary {}
impl AfterPort for Secondary {}

#[allow(dead_code)]
async fn primary_queries(provider: &crate::ServiceProvider) {
    let _ = provider.get_service::<dyn BeforePort>().await;
    let _ = provider.get_service::<dyn AfterPort>().await;
}

fn node<T: Send + Sync + 'static>() -> &'static crate::graph::CompiledNode {
    crate::graph::plan::load()
        .graph
        .nodes
        .iter()
        .find(|node| node.identifier.service_type == ServiceType::create::<T>())
        .unwrap()
}

#[test]
fn injectable_compiles_execution_policy_and_exact_dependency_slots() {
    let graph = &crate::graph::plan::load().graph;
    let controller = node::<Controller>();
    assert!(matches!(controller.constructor, Constructor::Class(_)));
    assert_eq!(
        controller.identifier.service_key,
        Some(ServiceKey::Named("controller".into()))
    );
    assert_eq!(controller.common.lifetime, ServiceLifetime::Scoped);
    assert!(
        controller
            .common
            .source
            .file
            .ends_with("injectable_provider.rs")
    );
    assert!(controller.common.cleanup.is_none());
    assert_eq!(controller.dependencies.len(), 2);
    assert_eq!(
        node::<TransientController>().common.lifetime,
        ServiceLifetime::Transient
    );

    let database = &controller.dependencies[0];
    assert_eq!(database.label, Some("database"));
    assert_eq!(database.slot, InputSlot::new(0));
    assert_eq!(
        database.requested,
        ServiceIdentifier::from(ServiceType::create::<Database>())
    );
    assert!(!database.optional);
    assert_eq!(
        graph.nodes[database.input.target().unwrap()].identifier,
        database.requested
    );

    let audit = &controller.dependencies[1];
    assert_eq!(audit.label, Some("audit"));
    assert_eq!(audit.slot, InputSlot::new(1));
    assert_eq!(
        audit.requested,
        ServiceIdentifier::new(
            Some(ServiceKey::Named("audit".into())),
            ServiceType::create::<dyn Audit>()
        )
    );
    assert!(audit.optional);
    assert!(audit.input.target().is_none());
    let crate::graph::DependencyInput::Absent(crate::graph::AbsentInput::Immediate(prepare)) =
        audit.input
    else {
        panic!("缺席的普通字段必须交付 Option<Injection<T>>")
    };
    assert!(
        prepare(audit.slot, None)
            .unwrap()
            .into_optional::<dyn Audit>(audit.slot)
            .unwrap()
            .is_none()
    );

    let tuple = node::<TupleConsumer>();
    assert_eq!(tuple.dependencies.len(), 1);
    let input = &tuple.dependencies[0];
    assert_eq!(input.label, None);
    assert_eq!(input.slot, InputSlot::new(0));
    assert_eq!(
        input.requested,
        ServiceIdentifier::new(
            Some(ServiceKey::Indexed(7)),
            ServiceType::create::<Database>()
        )
    );
    assert_eq!(
        graph.nodes[input.input.target().unwrap()].identifier,
        input.requested
    );
}

#[test]
fn generated_value_expressions_cleanup_and_primary_keep_their_business_semantics() {
    let Constructor::Class(constructor) = node::<FieldValues>().constructor else {
        panic!("class")
    };
    let erased = constructor(ConstructionInputs::empty()).unwrap();
    let values = match erased.downcast::<FieldValues>() {
        Ok(values) => values,
        Err(_) => panic!("FieldValues constructor result"),
    };
    assert_eq!(values.owned_literal, "123");
    assert_eq!(values.static_label, "from-static");
    assert_eq!(values.expression, 42);
    assert!(values.defaults.is_empty());
    drop(node::<CleanupController>()
        .common
        .cleanup
        .expect("cleanup callback")());

    let graph = &crate::graph::plan::load().graph;
    for (port, selected) in [
        (
            ServiceType::create::<dyn BeforePort>(),
            ServiceType::create::<PrimaryBeforeInjectable>(),
        ),
        (
            ServiceType::create::<dyn AfterPort>(),
            ServiceType::create::<InjectableBeforePrimary>(),
        ),
    ] {
        let route = &graph.routes[&ServiceIdentifier::from(port)];
        assert_eq!(
            graph.nodes[route.provider].identifier.service_type,
            selected
        );
        assert!(route.projection.is_some());
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.identifier.service_type == selected)
                .count(),
            1
        );
    }
}
