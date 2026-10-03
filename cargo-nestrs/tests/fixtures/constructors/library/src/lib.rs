//! 上游业务库保留私有构造实现；下游只依赖公开服务类型和普通查询方法。
use nestrs::{constructor, injectable};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static CLOCKS: AtomicUsize = AtomicUsize::new(0);
static REPOSITORIES: AtomicUsize = AtomicUsize::new(500);

fn next_repository() -> usize {
    REPOSITORIES.fetch_add(1, Ordering::SeqCst)
}

#[injectable]
pub struct Clock {
    sequence: usize,
}
impl Clock {
    #[constructor]
    fn create() -> Self {
        Self {
            sequence: CLOCKS.fetch_add(1, Ordering::SeqCst),
        }
    }
    pub fn sequence(&self) -> usize {
        self.sequence
    }
}

#[injectable]
pub struct Repository<T: Send + Sync + 'static> {
    clock: Clock,
    sequence: usize,
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Repository<T> {
    #[constructor]
    fn create(clock_service: Clock) -> Self {
        // 跨 crate 的闭合泛型需要保留私有 helper、static 与参数到字段的别名关系。
        let clock = clock_service;
        Self {
            clock,
            sequence: next_repository(),
            marker: PhantomData,
        }
    }
    pub fn sequence(&self) -> usize {
        self.sequence
    }
    pub fn clock_sequence(&self) -> usize {
        self.clock.sequence()
    }
}

pub trait RepositoryPort: Send + Sync {
    fn sequence(&self) -> usize;
}
impl<T: Send + Sync + 'static> RepositoryPort for Repository<T> {
    fn sequence(&self) -> usize {
        self.sequence
    }
}

#[injectable]
pub struct GenericWrapper<T: Send + Sync + 'static> {
    dependency: T,
}
impl<T: Send + Sync + 'static> GenericWrapper<T> {
    #[constructor]
    fn create(service: T) -> Self {
        Self {
            dependency: service,
        }
    }
    pub fn get(&self) -> &T {
        &self.dependency
    }
}

// 下游只能命名 Outer<Order>，不能直接命名或注册 Inner<Order>。构造参数仍须
// 为工具链贡献完整的泛型依赖路径，而不是依赖旧字段 #[inject] 提供的路径。
#[injectable]
pub struct Outer<T: Send + Sync + 'static> {
    inner: Inner<T>,
}
impl<T: Send + Sync + 'static> Outer<T> {
    #[constructor]
    fn create(inner: Inner<T>) -> Self {
        Self { inner }
    }
    pub fn clock_sequence(&self) -> usize {
        self.inner.clock.sequence()
    }
}

#[injectable]
struct Inner<T: Send + Sync + 'static> {
    clock: Clock,
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Inner<T> {
    #[constructor]
    fn create(clock: Clock) -> Self {
        Self {
            clock,
            marker: PhantomData,
        }
    }
}

pub trait OuterPort: Send + Sync {
    fn clock_sequence(&self) -> usize;
}
impl<T: Send + Sync + 'static> OuterPort for Outer<T> {
    fn clock_sequence(&self) -> usize {
        self.clock_sequence()
    }
}
