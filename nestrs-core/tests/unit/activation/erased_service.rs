use super::ErasedService;
use crate::{
    activation::{DependencyLease, ReleaseDomain},
    service::ServiceType,
};

#[test]
fn erased_service_ref_validates_the_concrete_type_before_casting() {
    let lease = DependencyLease::new(ErasedService::new(7_u32), vec![], ReleaseDomain::new());
    let reference = lease.erased_ref();
    let (pointer, retained) = reference.clone().cast::<u32>().unwrap();
    assert_eq!(Some(pointer), lease.pointer::<u32>());
    assert!(
        matches!(reference.cast::<u64>(), Err(actual) if actual == ServiceType::create::<u32>())
    );
    drop(lease);
    // SAFETY: retained 仍拥有对应分配的强 lease，释放原 lease 不会使指针悬垂。
    assert_eq!(unsafe { *pointer.as_ref() }, 7);
    drop(retained);
}

#[test]
fn moving_the_envelope_keeps_its_typed_address_stable() {
    let service = ErasedService::new(String::from("stable"));
    let before = service.pointer::<String>().unwrap();
    let moved = Box::new(service);
    assert_eq!(moved.pointer::<String>(), Some(before));
    assert!(moved.pointer::<str>().is_none());
    assert_eq!(moved.downcast::<String>().ok().as_deref(), Some("stable"));
}
