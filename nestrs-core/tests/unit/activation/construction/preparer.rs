use super::{prepare_bound_required, prepare_optional, prepare_optional_absent, prepare_required};
use crate::activation::{
    ConstructionError, DependencyLease, ErasedService, ErasedServiceRef, InputSlot, ReleaseDomain,
};

struct Alpha;
struct Beta;

fn erased<T>(value: T) -> ErasedServiceRef
where
    T: Send + Sync + 'static,
{
    DependencyLease::new(ErasedService::new(value), vec![], ReleaseDomain::new()).erased_ref()
}

#[test]
fn required_preparer_reports_a_missing_dependency_without_a_buffer() {
    assert!(matches!(
        prepare_required::<Alpha>(InputSlot::new(0), None),
        Err(ConstructionError::RequiredDependencyAbsent { slot }) if slot == InputSlot::new(0)
    ));
}

#[test]
fn preparer_maps_erased_type_mismatch_to_the_request_slot() {
    assert!(matches!(
        prepare_optional::<Beta>(InputSlot::new(3), Some(erased(Alpha))),
        Err(ConstructionError::InputTypeMismatch { slot, expected, actual })
            if slot == InputSlot::new(3)
                && expected == std::any::type_name::<Beta>()
                && actual == std::any::type_name::<Alpha>()
    ));
}

#[test]
fn bound_projection_retains_its_owner_and_the_exact_trait_vtable() {
    trait Port: Send + Sync {
        fn value(&self) -> u32;
    }
    struct Adapter(u32);
    impl Port for Adapter {
        fn value(&self) -> u32 {
            self.0
        }
    }
    let input = erased(Adapter(73));
    let prepared =
        prepare_bound_required::<Adapter, dyn Port>(InputSlot::new(0), Some(input), |adapter| {
            adapter
        })
        .unwrap();
    let token = prepared
        .into_required::<dyn Port>(InputSlot::new(0))
        .unwrap();
    assert_eq!(token.value(), 73);
    assert!(matches!(
        prepare_optional_absent::<dyn Port>(InputSlot::new(2), Some(erased(Adapter(1)))),
        Err(ConstructionError::UnprojectedTraitInput { slot, .. }) if slot == InputSlot::new(2)
    ));
}
