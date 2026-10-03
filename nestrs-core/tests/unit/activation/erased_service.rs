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
fn moved_and_failed_downcast_envelope_resolves_from_its_published_value() {
    let service = ErasedService::new(String::from("stable"));
    // 构造结果在 worker / 结果消息 / journal 之间移动时不能缓存早先借出的指针。
    // 失败的 downcast 也会移动 Box；之后仍应能从最终 lease 中恢复准确类型。
    let moved = Box::new(service);
    let Err(service) = moved.downcast::<u64>() else {
        panic!("wrong concrete type must not be accepted");
    };
    let lease = DependencyLease::new(service, vec![], ReleaseDomain::new());
    assert!(lease.pointer::<str>().is_none());
    assert!(lease.pointer::<u64>().is_none());
    let pointer = lease.pointer::<String>().unwrap();
    let retained = lease.clone();
    drop(lease);
    // SAFETY: retained 固定同一个最终实例；此后只移动 lease，不移动服务 Box。
    let value = unsafe { pointer.as_ref() };
    let repeated = retained.pointer::<String>().unwrap();
    assert_eq!(pointer, repeated);
    // SAFETY: 再次恢复指针只产生共享借用，不撤销仍合法的 value 借用。
    assert_eq!(unsafe { repeated.as_ref() }, value);
    assert_eq!(value, "stable");
    drop(retained);
}

#[test]
fn published_value_resolves_across_threads_and_drops_once() {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Tracked(Arc<AtomicUsize>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let drops = Arc::new(AtomicUsize::new(0));
    let service = ErasedService::new(Tracked(drops.clone()));
    let lease = DependencyLease::new(service, vec![], ReleaseDomain::new());
    let retained = lease.clone();
    std::thread::spawn(move || {
        let pointer = retained.pointer::<Tracked>().unwrap();
        // SAFETY: retained 在读取期间保活未移动的最终服务。
        assert_eq!(unsafe { pointer.as_ref() }.0.load(Ordering::SeqCst), 0);
    })
    .join()
    .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(lease);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn zero_sized_values_keep_exact_type_and_unpublished_downcast() {
    struct Empty;
    let lease = DependencyLease::new(ErasedService::new(Empty), vec![], ReleaseDomain::new());
    let pointer = lease.pointer::<Empty>().unwrap();
    assert!(lease.pointer::<()>().is_none());
    // SAFETY: ZST 指针同样来自真实类型的共享借用，并由 lease 保活。
    let _: &Empty = unsafe { pointer.as_ref() };
    assert!(matches!(
        ErasedService::new(Empty).downcast::<Empty>(),
        Ok(Empty)
    ));
}

#[test]
fn unsized_requests_require_the_exact_stored_type_not_a_possible_coercion() {
    trait Port: Send + Sync {
        fn value(&self) -> u32;
    }
    impl Port for u32 {
        fn value(&self) -> u32 {
            *self
        }
    }

    let concrete = DependencyLease::new(ErasedService::new(7_u32), vec![], ReleaseDomain::new());
    // 即使 concrete 可以合法 coercion 为 dyn Port，擦除容器也不能伪造投影。
    assert!(concrete.pointer::<dyn Port>().is_none());
    assert!(concrete.pointer::<[u8]>().is_none());

    let boxed: Box<dyn Port> = Box::new(11_u32);
    let boxed = DependencyLease::new(ErasedService::new(boxed), vec![], ReleaseDomain::new());
    assert!(boxed.pointer::<dyn Port>().is_none());
    assert!(boxed.pointer::<u32>().is_none());
    let pointer = boxed.pointer::<Box<dyn Port>>().unwrap();
    // SAFETY: boxed 在整个读取期间保活真实的 Box<dyn Port> 载荷。
    assert_eq!(unsafe { pointer.as_ref() }.value(), 11);
}
