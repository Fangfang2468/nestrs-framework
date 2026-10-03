//! 用旧字段布局作同一编译目标下的大小对照，不把一次性共享计划计作零成本。
//! 这不是跨 Rust 版本的 ABI 承诺；测试验证每个实际字段不再携带整份声明。

use std::{
    mem::size_of,
    sync::{Mutex, Weak},
};
use tokio::sync::OnceCell;

use super::{DeferredSlot, LazyReceiver, LazyResolver};
use crate::{
    ResolveError,
    activation::{Injection, InputSlot, LazyInputPlan},
    service::{ServiceIdentifier, ServiceSource},
};

// 原结构按同样字段类型重建，保留当前编译器和 target 的真实对齐方式。
#[allow(dead_code)]
struct PreviousDependency {
    resolver: Weak<dyn LazyResolver>,
    check_wait_allowed: fn() -> Result<(), &'static str>,
    provider: usize,
    consumer: ServiceIdentifier,
    source: ServiceSource,
    label: Option<&'static str>,
    preparer: fn(), // 历史单个函数指针的布局；不恢复已删除的构造协议。
    optional: bool,
}

#[allow(dead_code)]
struct PreviousSlot<T: ?Sized> {
    dependency: PreviousDependency,
    input: InputSlot,
    receiver: Mutex<Option<LazyReceiver>>,
    resolved: OnceCell<Result<Injection<T>, ResolveError>>,
}

trait Port: Send + Sync {}

#[test]
fn shared_descriptions_reduce_each_occurrence_without_erasing_dynamic_state() {
    let before = size_of::<PreviousSlot<u32>>();
    let after = size_of::<DeferredSlot<u32>>();
    let trait_before = size_of::<PreviousSlot<dyn Port>>();
    let trait_after = size_of::<DeferredSlot<dyn Port>>();
    assert!(after < before, "固定描述应从每个实际字段移入共享计划");
    assert!(trait_after < trait_before);
    println!(
        "lazy slot payload bytes: concrete {before} -> {after}, trait {trait_before} -> {trait_after}; shared plan payload per declared edge: {}",
        size_of::<LazyInputPlan>(),
    );
}
