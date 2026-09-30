use super::super::preparation::InputBuffer;
use super::super::slot::InputSlot;
use super::{ConstructionInputs, PreparedInput};
use crate::activation::{
    DependencyLease, ErasedService, Injection, ReleaseDomain,
    construction::error::ConstructionError, prepare_optional, prepare_required,
};

#[derive(Debug, Clone)]
struct Alpha;

#[derive(Debug, Clone)]
struct Beta;

#[derive(Debug, Clone)]
struct Tagged(u8);

fn required<T>(value: &mut T) -> PreparedInput
where
    T: Clone + Send + Sync + 'static,
{
    let lease = DependencyLease::new(
        ErasedService::new(value.clone()),
        vec![],
        ReleaseDomain::new(),
    );
    prepare_required::<T>(InputSlot::new(0), Some(lease.erased_ref())).unwrap()
}

fn optional<T>(value: Option<&mut T>) -> PreparedInput
where
    T: Clone + Send + Sync + 'static,
{
    let input = value.map(|value| {
        DependencyLease::new(
            ErasedService::new(value.clone()),
            vec![],
            ReleaseDomain::new(),
        )
        .erased_ref()
    });
    prepare_optional::<T>(InputSlot::new(0), input).unwrap()
}

#[test]
fn accepts_out_of_order_preparation_and_consumes_each_slot_once() {
    let mut alpha = Alpha;
    let mut beta = Beta;
    let mut buffer = InputBuffer::new(2);

    buffer
        .insert(InputSlot::new(1), optional(Some(&mut beta)))
        .expect("second slot should accept the first preparation");
    buffer
        .insert(InputSlot::new(0), required(&mut alpha))
        .expect("first slot should accept a later preparation");

    let mut inputs = buffer.finish().expect("all slots are ready");
    let _: Injection<Alpha> = inputs
        .take(InputSlot::new(0))
        .expect("required slot should yield Alpha");
    let optional_beta: Option<Injection<Beta>> = inputs
        .take_optional(InputSlot::new(1))
        .expect("optional slot should yield Beta");
    assert!(optional_beta.is_some());
    inputs
        .ensure_all_consumed()
        .expect("all prepared slots should be consumed");
}

#[test]
fn rejects_an_out_of_bounds_slot_without_changing_the_buffer() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(1);

    assert_eq!(
        buffer.insert(InputSlot::new(1), required(&mut alpha)),
        Err(ConstructionError::SlotOutOfBounds {
            slot: InputSlot::new(1),
            slot_count: 1,
        })
    );
    assert!(matches!(
        buffer.finish(),
        Err(ConstructionError::UnfilledSlot { slot }) if slot == InputSlot::new(0)
    ));
}

#[test]
fn rejects_duplicate_preparation() {
    let mut first = Tagged(1);
    let mut second = Tagged(2);
    let mut buffer = InputBuffer::new(1);

    buffer
        .insert(InputSlot::new(0), required(&mut first))
        .expect("first preparation should succeed");
    assert_eq!(
        buffer.insert(InputSlot::new(0), required(&mut second)),
        Err(ConstructionError::SlotAlreadyPrepared {
            slot: InputSlot::new(0),
        })
    );
    let mut inputs = buffer.finish().expect("first prepared value must remain");
    let token = inputs
        .take::<Tagged>(InputSlot::new(0))
        .expect("first prepared value should remain");
    assert_eq!(token.0, 1);
}

#[test]
fn finish_rejects_the_first_unfilled_slot() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(2);
    buffer
        .insert(InputSlot::new(0), required(&mut alpha))
        .expect("first slot should be ready");

    assert!(matches!(
        buffer.finish(),
        Err(ConstructionError::UnfilledSlot { slot }) if slot == InputSlot::new(1)
    ));
}

#[test]
fn required_optional_mismatch_does_not_consume_the_slot() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(1);
    buffer
        .insert(InputSlot::new(0), optional(Some(&mut alpha)))
        .expect("optional slot should prepare");

    let mut inputs = buffer.finish().expect("slot should be complete");
    assert!(matches!(
        inputs.take::<Alpha>(InputSlot::new(0)),
        Err(ConstructionError::RequiredInputExpected { slot }) if slot == InputSlot::new(0)
    ));
    assert!(matches!(
        inputs.take_optional::<Alpha>(InputSlot::new(0)),
        Ok(Some(_))
    ));
}

#[test]
fn optional_required_mismatch_does_not_consume_the_slot() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(1);
    buffer
        .insert(InputSlot::new(0), required(&mut alpha))
        .expect("required slot should prepare");

    let mut inputs = buffer.finish().expect("slot should be complete");
    assert!(matches!(
        inputs.take_optional::<Alpha>(InputSlot::new(0)),
        Err(ConstructionError::OptionalInputExpected { slot }) if slot == InputSlot::new(0)
    ));
    assert!(inputs.take::<Alpha>(InputSlot::new(0)).is_ok());
}

#[test]
fn an_absent_optional_input_is_ready_not_unfilled() {
    let mut buffer = InputBuffer::new(1);
    buffer
        .insert(InputSlot::new(0), optional::<Alpha>(None))
        .expect("an absent optional dependency still prepares its slot");

    let mut inputs = buffer.finish().expect("absent optional slot is complete");
    assert!(matches!(
        inputs.take_optional::<Alpha>(InputSlot::new(0)),
        Ok(None)
    ));
    inputs
        .ensure_all_consumed()
        .expect("the optional slot should be consumed");
}

#[test]
fn type_mismatch_does_not_consume_the_slot() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(1);
    buffer
        .insert(InputSlot::new(0), required(&mut alpha))
        .expect("required slot should prepare");

    let mut inputs = buffer.finish().expect("slot should be complete");
    assert!(matches!(
        inputs.take::<Beta>(InputSlot::new(0)),
        Err(ConstructionError::InputTypeMismatch {
            slot,
            expected,
            actual,
        }) if slot == InputSlot::new(0)
            && expected == std::any::type_name::<Beta>()
            && actual == std::any::type_name::<Alpha>()
    ));
    assert!(inputs.take::<Alpha>(InputSlot::new(0)).is_ok());
}

#[test]
fn optional_type_mismatch_does_not_consume_the_slot() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(1);
    buffer
        .insert(InputSlot::new(0), optional(Some(&mut alpha)))
        .expect("optional slot should prepare");

    let mut inputs = buffer.finish().expect("slot should be complete");
    assert!(matches!(
        inputs.take_optional::<Beta>(InputSlot::new(0)),
        Err(ConstructionError::InputTypeMismatch {
            slot,
            expected,
            actual,
        }) if slot == InputSlot::new(0)
            && expected == std::any::type_name::<Beta>()
            && actual == std::any::type_name::<Alpha>()
    ));
    assert!(matches!(
        inputs.take_optional::<Alpha>(InputSlot::new(0)),
        Ok(Some(_))
    ));
}

#[test]
fn rejects_second_consumption_of_a_slot() {
    let mut alpha = Alpha;
    let mut buffer = InputBuffer::new(1);
    buffer
        .insert(InputSlot::new(0), required(&mut alpha))
        .expect("required slot should prepare");

    let mut inputs = buffer.finish().expect("slot should be complete");
    let _: Injection<Alpha> = inputs
        .take(InputSlot::new(0))
        .expect("first consumption should succeed");
    assert!(matches!(
        inputs.take::<Alpha>(InputSlot::new(0)),
        Err(ConstructionError::SlotAlreadyConsumed { slot }) if slot == InputSlot::new(0)
    ));
}

#[test]
fn reports_the_first_unconsumed_slot() {
    let mut alpha = Alpha;
    let mut beta = Beta;
    let mut buffer = InputBuffer::new(2);
    buffer
        .insert(InputSlot::new(0), required(&mut alpha))
        .expect("first slot should prepare");
    buffer
        .insert(InputSlot::new(1), required(&mut beta))
        .expect("second slot should prepare");

    let mut inputs = buffer.finish().expect("all slots should be complete");
    let _: Injection<Alpha> = inputs
        .take(InputSlot::new(0))
        .expect("first slot should consume");
    assert_eq!(
        inputs.ensure_all_consumed(),
        Err(ConstructionError::UnconsumedSlot {
            slot: InputSlot::new(1),
        })
    );
    let _: Injection<Beta> = inputs
        .take(InputSlot::new(1))
        .expect("the remaining slot should still be consumable");
    inputs
        .ensure_all_consumed()
        .expect("the failed check must not consume a slot");
}

#[test]
fn empty_inputs_are_already_fully_consumed() {
    let mut inputs = ConstructionInputs::empty();
    inputs
        .ensure_all_consumed()
        .expect("empty input set should be valid");
    assert!(matches!(
        inputs.take::<Alpha>(InputSlot::new(0)),
        Err(ConstructionError::SlotOutOfBounds {
            slot,
            slot_count: 0,
        }) if slot == InputSlot::new(0)
    ));
}
