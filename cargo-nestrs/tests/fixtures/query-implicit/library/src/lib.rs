//! 每条隐式路径使用独立查询类型。载体不是provider，不能靠服务方法扫描补根。
#![allow(dead_code)]
use nestrs::injectable;
use nestrs_core::ServiceProvider;
use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicUsize, Ordering},
};

static BUILDS: AtomicUsize = AtomicUsize::new(0);
static QUERIES: AtomicUsize = AtomicUsize::new(0);
#[injectable]
struct Repository<T: Send + Sync + 'static> {
    #[value(BUILDS.fetch_add(1,Ordering::SeqCst))]
    id: usize,
    marker: PhantomData<T>,
}
pub fn builds() -> usize {
    BUILDS.load(Ordering::SeqCst)
}
pub fn queries() -> usize {
    QUERIES.load(Ordering::SeqCst)
}
fn query<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async {
            let _ = provider
                .get_required_service::<Repository<T>>()
                .await
                .unwrap();
            QUERIES.fetch_add(1, Ordering::SeqCst);
        })
    });
}

pub struct Read<'a, T> {
    provider: &'a ServiceProvider,
    value: usize,
    marker: PhantomData<T>,
}
impl<'a, T> Read<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self {
            provider,
            value: 17,
            marker: PhantomData,
        }
    }
}
impl<T: Send + Sync + 'static> Deref for Read<'_, T> {
    type Target = usize;
    fn deref(&self) -> &usize {
        query::<T>(self.provider);
        &self.value
    }
}
pub struct Outer<'a, T>(Read<'a, T>);
impl<'a, T> Outer<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(Read::new(provider))
    }
}
impl<'a, T: Send + Sync + 'static> Deref for Outer<'a, T> {
    type Target = Read<'a, T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct Shared<T>(PhantomData<T>);
pub struct Mutable<T>(PhantomData<T>);
pub struct Write<'a, T>(Read<'a, T>);
impl<'a, T> Write<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(Read::new(provider))
    }
}
impl<T: Send + Sync + 'static> Deref for Write<'_, T> {
    type Target = usize;
    fn deref(&self) -> &usize {
        query::<Shared<T>>(self.0.provider);
        &self.0.value
    }
}
impl<T: Send + Sync + 'static> DerefMut for Write<'_, T> {
    fn deref_mut(&mut self) -> &mut usize {
        query::<Mutable<T>>(self.0.provider);
        &mut self.0.value
    }
}

pub struct OuterMut<'a, T>(Write<'a, T>);
impl<'a, T> OuterMut<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(Write::new(provider))
    }
}
impl<'a, T: Send + Sync + 'static> Deref for OuterMut<'a, T> {
    type Target = Write<'a, T>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl<T: Send + Sync + 'static> DerefMut for OuterMut<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

pub struct OnDrop<'a, T: Send + Sync + 'static> {
    provider: &'a ServiceProvider,
    marker: PhantomData<T>,
}
impl<'a, T: Send + Sync + 'static> OnDrop<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self {
            provider,
            marker: PhantomData,
        }
    }
}
impl<T: Send + Sync + 'static> Drop for OnDrop<'_, T> {
    fn drop(&mut self) {
        query::<T>(self.provider);
    }
}
pub fn generic_drop<T>(value: T) {
    drop(value);
}
pub fn argument_drop<T>(_: T) {}

struct OpaqueDrop;
pub struct Opaque<'a> {
    hidden: OnDrop<'a, OpaqueDrop>,
}
pub fn opaque(provider: &ServiceProvider) -> Opaque<'_> {
    Opaque {
        hidden: OnDrop::new(provider),
    }
}

pub struct Recursive<'a, T: Send + Sync + 'static> {
    next: Option<Box<Recursive<'a, T>>>,
    value: OnDrop<'a, T>,
}
impl<'a, T: Send + Sync + 'static> Recursive<'a, T> {
    pub fn new(provider: &'a ServiceProvider, depth: usize) -> Self {
        Self {
            next: (depth > 0).then(|| Box::new(Self::new(provider, depth - 1))),
            value: OnDrop::new(provider),
        }
    }
}

struct UpstreamDeadDeref;
struct UpstreamDeadDrop;
fn upstream_dead(provider: &ServiceProvider) {
    if false {
        let query = Read::<UpstreamDeadDeref>::new(provider);
        let _: &usize = &query;
        let _query = OnDrop::<UpstreamDeadDrop>::new(provider);
    }
}
#[cfg(feature = "extra-root")]
struct FeatureDeadDeref;
#[cfg(feature = "extra-root")]
fn feature_dead(provider: &ServiceProvider) {
    if false {
        let query = Read::<FeatureDeadDeref>::new(provider);
        let _: &usize = &query;
    }
}

struct Unregistered;
#[injectable]
struct InvalidRepository<T: Send + Sync + 'static> {
    #[inject]
    missing: Unregistered,
    marker: PhantomData<T>,
}
pub struct InvalidDeref<'a, T>(&'a ServiceProvider, PhantomData<T>);
impl<'a, T> InvalidDeref<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(provider, PhantomData)
    }
}
impl<T: Send + Sync + 'static> Deref for InvalidDeref<'_, T> {
    type Target = usize;
    fn deref(&self) -> &usize {
        drop(self.0.get_required_service::<InvalidRepository<T>>());
        &17
    }
}
pub struct InvalidDrop<'a, T: Send + Sync + 'static>(&'a ServiceProvider, PhantomData<T>);
impl<'a, T: Send + Sync + 'static> InvalidDrop<'a, T> {
    pub fn new(provider: &'a ServiceProvider) -> Self {
        Self(provider, PhantomData)
    }
}
impl<T: Send + Sync + 'static> Drop for InvalidDrop<'_, T> {
    fn drop(&mut self) {
        drop(self.0.get_required_service::<InvalidRepository<T>>());
    }
}

// 关联字段只能通过实际 drop glue 选择，不能把同 trait 的其他 impl 变成根。
pub trait Family {
    type Field<'a>;
}
pub struct First;
pub struct Second;
pub struct Fixed;
pub struct Invalid;
pub struct AssociatedFirst;
pub struct AssociatedSecond;
pub struct AssociatedFixed;
impl Family for First {
    type Field<'a> = OnDrop<'a, AssociatedFirst>;
}
impl Family for Second {
    type Field<'a> = OnDrop<'a, AssociatedSecond>;
}
impl Family for Fixed {
    type Field<'a> = OnDrop<'a, AssociatedFixed>;
}
impl Family for Invalid {
    type Field<'a> = InvalidDrop<'a, u16>;
}
pub struct Associated<'a, T: Family> {
    field: T::Field<'a>,
}
pub struct FixedAssociated<'a> {
    field: <Fixed as Family>::Field<'a>,
}
pub fn associated_pair(
    provider: &ServiceProvider,
) -> (Associated<'_, First>, Associated<'_, Second>) {
    (
        Associated {
            field: OnDrop::new(provider),
        },
        Associated {
            field: OnDrop::new(provider),
        },
    )
}
pub fn fixed_associated(provider: &ServiceProvider) -> FixedAssociated<'_> {
    FixedAssociated {
        field: OnDrop::new(provider),
    }
}
pub fn invalid_associated(provider: &ServiceProvider) -> Associated<'_, Invalid> {
    Associated {
        field: InvalidDrop::new(provider),
    }
}
