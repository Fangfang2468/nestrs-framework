use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::super::slot::InputSlot;
use super::{ConstructionInput, ConstructionInputs, InputKind};
use crate::{
    activation::{
        ConstructionError, DependencyLease, ErasedService, Injection, ReleaseDomain, project_bound,
        project_required,
    },
    service::ServiceType,
};

#[derive(Debug, Clone)]
struct Alpha;
#[derive(Debug, Clone)]
struct Beta;
#[derive(Debug, Clone)]
struct Tagged(u8);

fn lease<T: Send + Sync + 'static>(value: T) -> DependencyLease {
    DependencyLease::new(ErasedService::new(value), vec![], ReleaseDomain::new())
}

fn required<T: Send + Sync + 'static>(value: T) -> ConstructionInput {
    ConstructionInput::immediate(
        ServiceType::create::<T>(),
        InputKind::Required,
        lease(value),
        project_required::<T>,
    )
}

fn optional<T: Send + Sync + 'static>(value: Option<T>) -> ConstructionInput {
    match value {
        Some(value) => ConstructionInput::immediate(
            ServiceType::create::<T>(),
            InputKind::Optional,
            lease(value),
            project_required::<T>,
        ),
        None => ConstructionInput::absent(ServiceType::create::<T>(), InputKind::Optional),
    }
}

#[test]
fn complete_inputs_can_be_consumed_out_of_order_once() {
    let mut inputs = ConstructionInputs::new(vec![required(Alpha), optional(Some(Beta))]).unwrap();
    let optional_beta: Option<Injection<Beta>> = inputs.take_optional(InputSlot::new(1)).unwrap();
    assert!(optional_beta.is_some());
    let _: Injection<Alpha> = inputs.take(InputSlot::new(0)).unwrap();
    inputs.ensure_all_consumed().unwrap();
}

#[test]
fn rejects_an_out_of_bounds_slot_without_consuming_valid_inputs() {
    let mut inputs = ConstructionInputs::new(vec![required(Tagged(1))]).unwrap();
    assert!(
        matches!(inputs.take::<Tagged>(InputSlot::new(1)), Err(ConstructionError::SlotOutOfBounds { slot, slot_count: 1 }) if slot == InputSlot::new(1))
    );
    assert_eq!(inputs.take::<Tagged>(InputSlot::new(0)).unwrap().0, 1);
}

#[test]
fn repeated_dependency_instances_are_distinct_slots() {
    let first = lease(Tagged(1));
    let mut inputs = ConstructionInputs::new(vec![
        ConstructionInput::immediate(
            ServiceType::create::<Tagged>(),
            InputKind::Required,
            first.clone(),
            project_required::<Tagged>,
        ),
        ConstructionInput::immediate(
            ServiceType::create::<Tagged>(),
            InputKind::Required,
            first,
            project_required::<Tagged>,
        ),
    ])
    .unwrap();
    let first = inputs.take::<Tagged>(InputSlot::new(0)).unwrap();
    let second = inputs.take::<Tagged>(InputSlot::new(1)).unwrap();
    assert!(std::ptr::eq(&*first, &*second));
    inputs.ensure_all_consumed().unwrap();
}

#[test]
fn required_optional_mismatch_does_not_consume_the_slot() {
    let mut inputs = ConstructionInputs::new(vec![optional(Some(Alpha))]).unwrap();
    assert!(
        matches!(inputs.take::<Alpha>(InputSlot::new(0)), Err(ConstructionError::RequiredInputExpected { slot }) if slot == InputSlot::new(0))
    );
    assert!(matches!(
        inputs.take_optional::<Alpha>(InputSlot::new(0)),
        Ok(Some(_))
    ));
}

#[test]
fn optional_required_mismatch_does_not_consume_the_slot() {
    let mut inputs = ConstructionInputs::new(vec![required(Alpha)]).unwrap();
    assert!(
        matches!(inputs.take_optional::<Alpha>(InputSlot::new(0)), Err(ConstructionError::OptionalInputExpected { slot }) if slot == InputSlot::new(0))
    );
    assert!(inputs.take::<Alpha>(InputSlot::new(0)).is_ok());
}

#[test]
fn an_absent_optional_input_is_ready_not_unfilled() {
    let mut inputs = ConstructionInputs::new(vec![optional::<Alpha>(None)]).unwrap();
    assert!(
        inputs
            .take_optional::<Alpha>(InputSlot::new(0))
            .unwrap()
            .is_none()
    );
    inputs.ensure_all_consumed().unwrap();
}

#[test]
fn type_mismatch_does_not_consume_the_slot() {
    let mut inputs = ConstructionInputs::new(vec![required(Alpha)]).unwrap();
    assert!(
        matches!(inputs.take::<Beta>(InputSlot::new(0)), Err(ConstructionError::InputTypeMismatch { slot, expected, actual })
        if slot == InputSlot::new(0) && expected == std::any::type_name::<Beta>() && actual == std::any::type_name::<Alpha>())
    );
    assert!(inputs.take::<Alpha>(InputSlot::new(0)).is_ok());
}

#[test]
fn optional_type_mismatch_does_not_consume_the_slot() {
    for input in [optional(Some(Alpha)), optional::<Alpha>(None)] {
        let mut inputs = ConstructionInputs::new(vec![input]).unwrap();
        assert!(
            matches!(inputs.take_optional::<Beta>(InputSlot::new(0)), Err(ConstructionError::InputTypeMismatch { expected, actual, .. })
            if expected == std::any::type_name::<Beta>() && actual == std::any::type_name::<Alpha>())
        );
        inputs.take_optional::<Alpha>(InputSlot::new(0)).unwrap();
        inputs.ensure_all_consumed().unwrap();
    }
}

#[test]
fn absent_lazy_preserves_type_and_delivery_kind_before_consumption() {
    let mut inputs = ConstructionInputs::new(vec![ConstructionInput::absent(
        ServiceType::create::<Alpha>(),
        InputKind::LazyOptional,
    )])
    .unwrap();
    assert!(matches!(
        inputs.take_optional::<Alpha>(InputSlot::new(0)),
        Err(ConstructionError::OptionalInputExpected { .. })
    ));
    assert!(matches!(
        inputs.take_optional_lazy::<Beta>(InputSlot::new(0)),
        Err(ConstructionError::InputTypeMismatch { .. })
    ));
    assert!(
        inputs
            .take_optional_lazy::<Alpha>(InputSlot::new(0))
            .unwrap()
            .is_none()
    );
    inputs.ensure_all_consumed().unwrap();
}

#[test]
fn rejects_second_consumption_of_a_slot() {
    let mut inputs = ConstructionInputs::new(vec![required(Alpha)]).unwrap();
    let _: Injection<Alpha> = inputs.take(InputSlot::new(0)).unwrap();
    assert!(
        matches!(inputs.take::<Alpha>(InputSlot::new(0)), Err(ConstructionError::SlotAlreadyConsumed { slot }) if slot == InputSlot::new(0))
    );
}

#[test]
fn reports_the_first_unconsumed_slot() {
    let mut inputs = ConstructionInputs::new(vec![required(Alpha), required(Beta)]).unwrap();
    let _: Injection<Alpha> = inputs.take(InputSlot::new(0)).unwrap();
    assert_eq!(
        inputs.ensure_all_consumed(),
        Err(ConstructionError::UnconsumedSlot {
            slot: InputSlot::new(1)
        })
    );
    let _: Injection<Beta> = inputs.take(InputSlot::new(1)).unwrap();
    inputs.ensure_all_consumed().unwrap();
}

#[test]
fn empty_inputs_are_already_fully_consumed() {
    let mut inputs = ConstructionInputs::empty();
    inputs.ensure_all_consumed().unwrap();
    assert!(
        matches!(inputs.take::<Alpha>(InputSlot::new(0)), Err(ConstructionError::SlotOutOfBounds { slot, slot_count: 0 }) if slot == InputSlot::new(0))
    );
}

#[test]
fn required_absence_is_rejected_when_building_the_complete_inputs() {
    for kind in [InputKind::Required, InputKind::LazyRequired] {
        assert!(
            matches!(ConstructionInputs::new(vec![ConstructionInput::absent(ServiceType::create::<Alpha>(), kind)]),
            Err(ConstructionError::RequiredDependencyAbsent { slot }) if slot == InputSlot::new(0))
        );
    }
}

#[test]
fn erased_type_mismatch_keeps_the_slot_and_reports_its_position() {
    let mut inputs = ConstructionInputs::new(vec![
        optional::<Alpha>(None),
        ConstructionInput::immediate(
            ServiceType::create::<Beta>(),
            InputKind::Required,
            lease(Alpha),
            project_required::<Beta>,
        ),
    ])
    .unwrap();
    for _ in 0..2 {
        assert!(
            matches!(inputs.take::<Beta>(InputSlot::new(1)), Err(ConstructionError::InputTypeMismatch { slot, expected, actual })
            if slot == InputSlot::new(1) && expected == std::any::type_name::<Beta>() && actual == std::any::type_name::<Alpha>())
        );
    }
    inputs.take_optional::<Alpha>(InputSlot::new(0)).unwrap();
    assert_eq!(
        inputs.ensure_all_consumed(),
        Err(ConstructionError::UnconsumedSlot {
            slot: InputSlot::new(1)
        })
    );
}

#[test]
fn failed_projection_keeps_original_input_available_for_retry() {
    use std::cell::Cell;
    thread_local! { static FAIL: Cell<bool> = const { Cell::new(true) }; }
    let input = ConstructionInput::immediate(
        ServiceType::create::<Tagged>(),
        InputKind::Required,
        lease(Tagged(73)),
        |slot, input, target| {
            if FAIL.with(Cell::get) {
                return Ok(());
            }
            project_required::<Tagged>(slot, input, target)
        },
    );
    let mut inputs = ConstructionInputs::new(vec![input]).unwrap();
    assert!(matches!(
        inputs.take::<Tagged>(InputSlot::new(0)),
        Err(ConstructionError::UnfilledSlot { .. })
    ));
    FAIL.with(|state| state.set(false));
    assert_eq!(inputs.take::<Tagged>(InputSlot::new(0)).unwrap().0, 73);
    inputs.ensure_all_consumed().unwrap();
}

#[test]
fn trait_input_retains_its_owner_and_the_exact_vtable() {
    trait Port: Send + Sync {
        fn value(&self) -> u32;
    }
    struct Adapter(u32, Arc<AtomicUsize>);
    impl Port for Adapter {
        fn value(&self) -> u32 {
            self.0
        }
    }
    impl Drop for Adapter {
        fn drop(&mut self) {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let input = ConstructionInput::immediate(
        ServiceType::create::<dyn Port>(),
        InputKind::Required,
        lease(Adapter(73, drops.clone())),
        |slot, input, target| {
            project_bound::<Adapter, dyn Port>(slot, input, target, |value| value)
        },
    );
    let mut inputs = ConstructionInputs::new(vec![input]).unwrap();
    let token = inputs.take::<dyn Port>(InputSlot::new(0)).unwrap();
    drop(inputs);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(token.value(), 73);
    drop(token);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn invalid_complete_input_releases_all_owned_dependencies() {
    struct Counted(Arc<AtomicUsize>);
    impl Drop for Counted {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let result = ConstructionInputs::new(vec![
        required(Counted(drops.clone())),
        ConstructionInput::absent(ServiceType::create::<Alpha>(), InputKind::Required),
        required(Counted(drops.clone())),
    ]);
    assert!(
        matches!(result, Err(ConstructionError::RequiredDependencyAbsent { slot }) if slot == InputSlot::new(1))
    );
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn a_direct_instance_cannot_masquerade_as_an_uninitialized_lazy_input() {
    for kind in [InputKind::LazyRequired, InputKind::LazyOptional] {
        let result = ConstructionInputs::new(vec![ConstructionInput::immediate(
            ServiceType::create::<Alpha>(),
            kind,
            lease(Alpha),
            project_required::<Alpha>,
        )]);
        assert!(result.is_err());
    }
}

#[test]
fn wrong_lazy_source_shape_or_slot_is_rejected_without_requesting_a_target() {
    use crate::activation::{
        LazyDependency, LazyInputPlan,
        deferred::{LazyReceiver, LazyResolver},
    };
    use crate::service::{ServiceIdentifier, ServiceSource};
    use std::sync::Weak;
    struct NoRequest;
    impl LazyResolver for NoRequest {
        fn request(&self, _: usize) -> Result<LazyReceiver, &'static str> {
            panic!("input assembly must never request a lazy target")
        }
    }
    let make = |slot| {
        let resolver: Weak<dyn LazyResolver> = Weak::<NoRequest>::new();
        LazyDependency {
            plan: Arc::new(LazyInputPlan {
                provider: 7,
                consumer: ServiceIdentifier::from(ServiceType::create::<Beta>()),
                source: ServiceSource::new(file!(), line!(), 1),
                label: Some("dependency"),
                input: InputSlot::new(slot),
                project: project_required::<Alpha>,
            }),
            resolver,
            check_wait_allowed: || Ok(()),
        }
    };
    assert!(matches!(
        ConstructionInputs::new(vec![ConstructionInput::lazy(
            ServiceType::create::<Alpha>(),
            InputKind::Required,
            make(0),
        )]),
        Err(ConstructionError::RequiredInputExpected { .. })
    ));
    assert!(
        matches!(ConstructionInputs::new(vec![ConstructionInput::lazy(
        ServiceType::create::<Alpha>(), InputKind::LazyRequired, make(1),
    )]), Err(ConstructionError::InputSlotMismatch { slot, actual })
        if slot == InputSlot::new(0) && actual == InputSlot::new(1))
    );
}
