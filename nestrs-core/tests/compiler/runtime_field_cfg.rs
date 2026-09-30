//! rustc 条件编译后的同一字段集合驱动类型、构造与依赖槽位。
use crate::activation::InputSlot;
use crate::registration::provider::{Provider, ProviderDefinition};
use crate::service::ServiceType;
use std::marker::PhantomData;

use nestrs::{injectable, primary};
use nestrs_core::{ServiceProvider, get_required_service};

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
    let named = get_required_service!(provider, Named).await.unwrap();
    assert_eq!(named.first.value, 37);
    assert_eq!(named.value, 12);
    assert_eq!(named.default, 0);
    assert_eq!(named.port.value(), 37);
    assert!(named.optional.is_none());

    let tuple = get_required_service!(provider, Tuple).await.unwrap();
    assert_eq!(tuple.0.value, 37);
    assert_eq!(tuple.1, 19);
    assert_eq!(tuple.2, 0);
    assert_eq!(tuple.3.value, 37);

    let generic = get_required_service!(provider, Generic<u32>).await.unwrap();
    assert_eq!(generic.dependency.value, 37);
    get_required_service!(provider, EmptyNamed).await.unwrap();
    get_required_service!(provider, EmptyTuple).await.unwrap();
    let generated = get_required_service!(provider, generated::MacroService)
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
fn configured_dependency_slots_are_contiguous_and_match_actual_field_positions() {
    for (service_type, positions) in [
        (ServiceType::create::<Named>(), vec![0, 3, 4]),
        (ServiceType::create::<Tuple>(), vec![0, 3]),
    ] {
        let provider = crate::registration::catalog::collect().providers.into_iter()
            .find(|provider| matches!(provider, Provider::Class(provider) if provider.provide.service_type == service_type))
            .unwrap();
        let Provider::Class(provider) = provider else {
            panic!("injectable produces a class provider");
        };
        assert_eq!(provider.dependencies.len(), positions.len());
        for (slot, (dependency, position)) in
            provider.dependencies.iter().zip(positions).enumerate()
        {
            assert_eq!(dependency.input_slot, InputSlot::new(slot));
            assert_eq!(dependency.declaration_position, position);
        }
    }
    let Provider::Class(generic) = <Generic<u32> as ProviderDefinition>::provider() else {
        panic!("generic injectable produces a class provider");
    };
    assert_eq!(generic.dependencies.len(), 1);
    assert_eq!(generic.dependencies[0].input_slot, InputSlot::new(0));
    assert_eq!(generic.dependencies[0].declaration_position, 1);
}
