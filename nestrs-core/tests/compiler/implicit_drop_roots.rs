//! core 的隔离 cfg(test) 契约也是业务编译单元，隐式 Drop 必须贡献闭合查询。
#![allow(dead_code)]

use crate::{ServiceProvider, service::ServiceType};
use nestrs::injectable;
use std::{marker::PhantomData, mem::ManuallyDrop};

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
}

struct Parameter;
struct Local;
struct Aggregate;
struct DeadBranch;
struct Forget;
struct Manual;
struct Reference;

struct DropQuery<'a, T: Send + Sync + 'static> {
    provider: &'a ServiceProvider,
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Drop for DropQuery<'_, T> {
    fn drop(&mut self) {
        drop(self.provider.get_required_service::<Repository<T>>());
    }
}

fn parameter(_: DropQuery<'_, Parameter>) {}
fn local_and_dead(provider: &ServiceProvider) {
    let _value = DropQuery::<Local> {
        provider,
        marker: PhantomData,
    };
    if false {
        let _value = DropQuery::<DeadBranch> {
            provider,
            marker: PhantomData,
        };
    }
}
struct Opaque<'a> {
    value: DropQuery<'a, Aggregate>,
}
fn aggregate(_: Opaque<'_>) {}

struct Missing;
#[injectable]
struct InvalidRepository<T> {
    #[inject]
    missing: Missing,
    marker: PhantomData<T>,
}
struct InvalidDrop<'a, T: Send + Sync + 'static> {
    provider: &'a ServiceProvider,
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Drop for InvalidDrop<'_, T> {
    fn drop(&mut self) {
        drop(self.provider.get_required_service::<InvalidRepository<T>>());
    }
}
fn suppressed(provider: &ServiceProvider) {
    std::mem::forget(InvalidDrop::<Forget> {
        provider,
        marker: PhantomData,
    });
    let _held = ManuallyDrop::new(InvalidDrop::<Manual> {
        provider,
        marker: PhantomData,
    });
    let _reference: Option<&InvalidDrop<'_, Reference>> = None;
}

#[test]
fn core_test_drop_queries_preserve_native_semantics_in_the_frozen_plan() {
    let graph = &crate::graph::plan::CompiledApplication::load().graph;
    let expected = [
        ServiceType::create::<Repository<Parameter>>(),
        ServiceType::create::<Repository<Local>>(),
        ServiceType::create::<Repository<Aggregate>>(),
        ServiceType::create::<Repository<DeadBranch>>(),
    ];
    assert_eq!(graph.nodes.len(), expected.len());
    for service_type in expected {
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.identifier.service_type == service_type)
                .count(),
            1,
            "implicit destructor query root missing or duplicated"
        );
    }
}
