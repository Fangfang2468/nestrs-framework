use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use super::{ProjectionTarget, ServiceProjection, project_bound, project_required};
use crate::activation::{
    ConstructionError, DependencyLease, ErasedService, ErasedServiceRef, InputSlot, ReleaseDomain,
    prepare_bound_required, prepare_required,
};

/// 只统计显式测量窗口内、当前测试线程的分配，不把并发测试/测试框架分配混入结果。
/// 所有实际分配仍委托 System；计数关闭时不改变生产分配与释放行为。
struct CountingAllocator;

thread_local! {
    static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_allocation() {
    // TLS 清理期间可能有系统释放/分配；try_with 避免已退出线程再次访问 TLS 而 panic。
    let _ = ALLOCATIONS.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

// SAFETY: 每个操作原样传递 System 所需的地址和布局；额外的线程局部计数不碰分配内存。
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: GlobalAlloc 调用者提供合法 layout，原样交给 System。
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: 与 alloc 相同，零初始化由 System 完成。
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: 指针和布局仍属于同一 System 分配器，没有转换或提前释放。
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: 原样交回此前分配内存的 System，计数器不持有任何相关内存。
        unsafe { System.dealloc(pointer, layout) }
    }
}

fn allocations<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ALLOCATIONS.with(|value| value.set(None));
        }
    }
    ALLOCATIONS.with(|value| assert!(value.replace(Some(0)).is_none()));
    let reset = Reset;
    let result = operation();
    let count = ALLOCATIONS.with(|value| value.get().unwrap());
    drop(reset);
    (result, count)
}

fn lease<T: Send + Sync + 'static>(value: T) -> DependencyLease {
    DependencyLease::new(ErasedService::new(value), vec![], ReleaseDomain::new())
}

trait Port: Send + Sync {
    fn value(&self) -> u32;
}

struct Adapter(u32);
impl Port for Adapter {
    fn value(&self) -> u32 {
        self.0
    }
}

#[test]
fn concrete_and_trait_projection_deliver_without_temporary_allocations() {
    // 实例、lease、释放域都提前准备好，窗口只覆盖首次投影交付这一条待优化路径。
    let instance = lease(Adapter(73));
    let slot = InputSlot::new(4);
    let (direct, direct_count) = allocations(|| {
        ProjectionTarget::project::<Adapter>(slot, instance.clone(), project_required::<Adapter>)
            .unwrap()
    });
    assert_eq!(direct.0, 73);
    assert_eq!(direct_count, 0);

    let (bound, bound_count) = allocations(|| {
        ProjectionTarget::project::<dyn Port>(slot, instance.clone(), |slot, input, target| {
            project_bound::<Adapter, dyn Port>(slot, input, target, |value| value)
        })
        .unwrap()
    });
    assert_eq!(bound.value(), 73);
    assert_eq!(bound_count, 0);

    // 与构造输入仍使用的通用 PreparedInput 路径作同条件对照；根 trait 查询和
    // LazyInjection 现在共用上面的直接交付，实例身份检查也不能引入临时堆载荷。
    let (ordinary, ordinary_count) = allocations(|| {
        let input = prepare_required::<Adapter>(slot, Some(instance.erased_ref())).unwrap();
        std::hint::black_box(input)
            .into_required::<Adapter>(slot)
            .unwrap()
    });
    assert_eq!(ordinary.0, 73);
    assert_eq!(ordinary_count, 1);

    let (ordinary_trait, ordinary_trait_count) = allocations(|| {
        let input = prepare_bound_required::<Adapter, dyn Port>(
            slot,
            Some(instance.erased_ref()),
            |value| value,
        )
        .unwrap();
        std::hint::black_box(input)
            .into_required::<dyn Port>(slot)
            .unwrap()
    });
    assert_eq!(ordinary_trait.value(), 73);
    assert_eq!(ordinary_trait_count, 1);
}

#[test]
fn wrong_erased_instance_and_wrong_projection_target_are_rejected() {
    let slot = InputSlot::new(7);
    assert!(matches!(
        ProjectionTarget::project::<Adapter>(slot, lease(17_u32), project_required::<Adapter>),
        Err(ConstructionError::InputTypeMismatch { slot: actual, expected, actual: ty })
            if actual == slot && expected == std::any::type_name::<Adapter>()
                && ty == std::any::type_name::<u32>()
    ));

    assert!(matches!(
        ProjectionTarget::project::<u32>(slot, lease(Adapter(17)), project_required::<Adapter>),
        Err(ConstructionError::InputTypeMismatch { slot: actual, expected, actual: ty })
            if actual == slot && expected == std::any::type_name::<u32>()
                && ty == std::any::type_name::<Adapter>()
    ));
}

#[test]
fn empty_delivery_and_swallowed_duplicate_writes_do_not_report_success() {
    let slot = InputSlot::new(2);
    assert!(matches!(
        ProjectionTarget::project::<Adapter>(slot, lease(Adapter(1)), |_, _, _| Ok(())),
        Err(ConstructionError::UnfilledSlot { slot: actual }) if actual == slot
    ));

    assert!(matches!(
        ProjectionTarget::project::<Adapter>(slot, lease(Adapter(1)), |slot, input, target| {
            project_required::<Adapter>(slot, input.clone(), target)?;
            let _ignored = project_required::<Adapter>(slot, input, target);
            Ok(())
        }),
        Err(ConstructionError::SlotAlreadyPrepared { slot: actual }) if actual == slot
    ));
}

#[test]
fn a_swallowed_wrong_type_write_invalidates_the_whole_delivery() {
    let slot = InputSlot::new(9);
    assert!(matches!(
        ProjectionTarget::project::<Adapter>(slot, lease(Adapter(1)), |slot, input, target| {
            let wrong = ServiceProjection::new(slot, lease(3_u32).erased_ref()).concrete::<u32>()?;
            let _ignored = target.write(wrong);
            let _ignored = project_required::<Adapter>(slot, input, target);
            Ok(())
        }),
        Err(ConstructionError::InputTypeMismatch { slot: actual, .. }) if actual == slot
    ));
}

struct TrackedAdapter(Arc<AtomicUsize>);

impl Port for TrackedAdapter {
    fn value(&self) -> u32 {
        91
    }
}

impl Drop for TrackedAdapter {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// 模拟完全由安全调用组成的错误 adapter：取得正确类型的输入，再创建同类型的
/// 另一个实例。它的令牌本身有效，但不属于执行计划已选择、owner 已发布的实例。
fn replacement_instance(
    slot: InputSlot,
    input: ErasedServiceRef,
) -> Result<ErasedServiceRef, ConstructionError> {
    let original = ServiceProjection::new(slot, input).concrete::<TrackedAdapter>()?;
    Ok(lease(TrackedAdapter(original.0.clone())).erased_ref())
}

#[test]
fn same_type_replacement_is_rejected_and_only_the_substitute_is_released() {
    let drops = Arc::new(AtomicUsize::new(0));
    let original = lease(TrackedAdapter(drops.clone()));
    let slot = InputSlot::new(11);
    let result = ProjectionTarget::project::<TrackedAdapter>(
        slot,
        original.clone(),
        |slot, input, target| {
            project_required::<TrackedAdapter>(slot, replacement_instance(slot, input)?, target)
        },
    );
    assert!(matches!(
        result,
        Err(ConstructionError::ProjectionOwnerMismatch { slot: actual }) if actual == slot
    ));
    // 拒绝错误投影时立即回滚替代实例；原实例的 lease 仍由调用者持有。
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    let correct = ProjectionTarget::project::<TrackedAdapter>(
        slot,
        original.clone(),
        project_required::<TrackedAdapter>,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&correct.0, &drops));
    drop(correct);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(original);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn valid_trait_coercion_cannot_hide_a_different_instance_owner() {
    let drops = Arc::new(AtomicUsize::new(0));
    let original = lease(TrackedAdapter(drops.clone()));
    let slot = InputSlot::new(12);
    let result =
        ProjectionTarget::project::<dyn Port>(slot, original.clone(), |slot, input, target| {
            project_bound::<TrackedAdapter, dyn Port>(
                slot,
                replacement_instance(slot, input)?,
                target,
                |value| value,
            )
        });
    assert!(matches!(
        result,
        Err(ConstructionError::ProjectionOwnerMismatch { slot: actual }) if actual == slot
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(original);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn projected_token_owns_the_instance_after_the_original_lease_is_gone() {
    struct Tracked(Arc<AtomicUsize>);
    impl Drop for Tracked {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let token = ProjectionTarget::project::<Tracked>(
        InputSlot::new(0),
        lease(Tracked(drops.clone())),
        project_required::<Tracked>,
    )
    .unwrap();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(Arc::ptr_eq(&token.0, &drops));
    drop(token);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}
