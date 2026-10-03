//! rustc 条件编译后的同一字段集合驱动类型、构造与依赖槽位。
use crate::activation::InputSlot;
use crate::service::ServiceType;
use std::marker::PhantomData;

use nestrs::{injectable, primary};
use nestrs_core::ServiceProvider;

trait Port: Send + Sync {
    fn value(&self) -> u32;
}

#[injectable]
struct Dependency {
    #[cfg(any())]
    absent: MissingDependencyField,
    #[value(37)]
    value: u32,
}

impl Port for Dependency {
    fn value(&self) -> u32 {
        self.value
    }
}

#[injectable]
#[primary]
struct Named {
    #[cfg(any())]
    #[inject]
    missing_injected: MissingInjected,
    #[cfg_attr(all(), cfg(any()))]
    missing_default: MissingDefault,
    #[cfg(any())]
    #[value(missing_expression())]
    missing_value: MissingValue,
    #[cfg(all())]
    #[inject]
    first: Dependency,
    #[cfg_attr(all(), value(12))]
    value: u32,
    #[cfg_attr(any(), inject)]
    default: u32,
    #[cfg_attr(all(), cfg_attr(all(), nestrs::inject))]
    port: dyn Port,
    #[cfg(any())]
    #[inject]
    #[value(123)]
    disabled_conflict: MissingConflict,
    #[cfg_attr(all(), nestrs::inject("absent"))]
    optional: Option<Dependency>,
}

#[injectable]
struct Tuple(
    #[cfg(any())] MissingDefault,
    #[cfg_attr(all(), inject)] Dependency,
    #[cfg(any())]
    #[inject]
    MissingInjected,
    #[cfg_attr(all(), value(19))] u32,
    #[cfg_attr(any(), value(unknown_expression()))] u32,
    #[cfg_attr(all(), cfg_attr(all(), inject))] Dependency,
    #[cfg(any())]
    #[value(unknown_expression())]
    MissingValue,
);

#[injectable]
struct Generic<T> {
    #[cfg(any())]
    #[inject]
    missing: MissingGeneric<T>,
    marker: PhantomData<T>,
    #[cfg_attr(all(), inject)]
    dependency: Dependency,
}

#[injectable]
struct EmptyNamed {
    #[cfg(any())]
    #[inject]
    missing: DoesNotExist,
}

#[injectable]
struct EmptyTuple(#[cfg(any())] DoesNotExist);

mod generated {
    use nestrs::injectable as service;

    macro_rules! declare {
        () => {
            #[service]
            #[derive(Debug)]
            pub struct MacroService {
                #[cfg_attr(all(), cfg(any()))]
                disabled: TypeNotAvailable,
                #[cfg_attr(debug_assertions, value("debug"))]
                #[cfg_attr(not(debug_assertions), value("release"))]
                pub profile: &'static str,
            }
        };
    }
    declare!();
}

#[tokio::test]
async fn configured_fields_construct_named_tuple_generic_and_empty_services() {
    let provider = ServiceProvider::build().await.unwrap();
    let named = provider.get_required_service::<Named>().await.unwrap();
    assert_eq!(named.first.value, 37);
    assert_eq!(named.value, 12);
    assert_eq!(named.default, 0);
    assert_eq!(named.port.value(), 37);
    assert!(named.optional.is_none());

    let tuple = provider.get_required_service::<Tuple>().await.unwrap();
    assert_eq!(tuple.0.value, 37);
    assert_eq!(tuple.1, 19);
    assert_eq!(tuple.2, 0);
    assert_eq!(tuple.3.value, 37);

    let generic = provider
        .get_required_service::<Generic<u32>>()
        .await
        .unwrap();
    assert_eq!(generic.dependency.value, 37);
    provider.get_required_service::<EmptyNamed>().await.unwrap();
    provider.get_required_service::<EmptyTuple>().await.unwrap();
    let generated = provider
        .get_required_service::<generated::MacroService>()
        .await
        .unwrap();
    assert_eq!(
        generated.profile,
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    assert!(format!("{generated:?}").contains(generated.profile));
    provider.dispose_async().await.unwrap();
}

#[test]
fn configured_dependency_slots_are_contiguous_and_preserve_requested_fields() {
    let graph = &crate::graph::plan::CompiledApplication::load().graph;
    for (service_type, labels) in [
        (
            ServiceType::create::<Named>(),
            vec![Some("first"), Some("port"), Some("optional")],
        ),
        (ServiceType::create::<Tuple>(), vec![None, None]),
        (
            ServiceType::create::<Generic<u32>>(),
            vec![Some("dependency")],
        ),
    ] {
        let node = graph
            .nodes
            .iter()
            .find(|node| node.identifier.service_type == service_type)
            .unwrap();
        assert!(matches!(
            node.constructor,
            crate::graph::Constructor::Class(_)
        ));
        assert_eq!(node.dependencies.len(), labels.len());
        for (slot, (dependency, label)) in node.dependencies.iter().zip(labels).enumerate() {
            assert_eq!(dependency.slot, InputSlot::new(slot));
            assert_eq!(dependency.label, label);
        }
    }
    let named = graph
        .nodes
        .iter()
        .find(|node| node.identifier.service_type == ServiceType::create::<Named>())
        .unwrap();
    assert!(named.dependencies[0].input.target().is_some());
    assert!(named.dependencies[1].input.target().is_some());
    assert!(named.dependencies[2].input.target().is_none());
}
