//! 已构造服务的类型擦除容器与带 lease 的构造期引用。
//!
//! 运行时可以统一存放不同服务，但不能因此丢失准确类型信息。服务值放进 Box，类型化
//! 地址恢复器引用类型关联的静态操作；恢复器不缓存从尚可能移动的 Box 派生的裸指针。
//! 实例被 lease 固定在共享记录之后，才从当前值的共享借用恢复准确地址。

use std::{any::Any, ptr::NonNull};

use super::DependencyLease;
use crate::service::{Injectable, ServiceType};

/// 拥有 concrete 服务值的类型擦除 Box；其准确类型由外层记录保留。
type AnyService = Box<dyn Any + Send + Sync>;

/// 保留准确 T 的恢复能力，支持 pointer 的 ?Sized 请求而不伪造宽指针。
/// 函数指针天然 Send + Sync，不持有服务地址，也不需要手写自动 trait 的安全实现。
struct TypedAddressResolver<T: ?Sized>(fn(&AnyService) -> Option<NonNull<T>>);

impl<T: Injectable> TypedAddressResolver<T> {
    /// 仅含函数指针、无内部可变性或析构的关联常量可以被提升为静态引用。
    /// 每个 envelope 只借用它，不为相同的恢复能力再分配一个 Box。
    const SHARED: Self = Self(|value| value.downcast_ref::<T>().map(NonNull::from));
}

/// 拥有服务值的类型擦除容器；发布后由实例记录持有，不能再移出其中的值。
pub struct ErasedService {
    /// 该值或输入的准确 Rust 类型身份，用于交付前核对。
    service_type: ServiceType,

    /// 拥有的 concrete 服务值，发布后不再从实例记录移出。
    value: AnyService,

    /// 类型关联的静态恢复能力，不缓存服务移动前派生的地址。
    address_resolver: &'static (dyn Any + Send + Sync),
}

impl ErasedService {
    /// 擦除一个成功构造的 concrete service，记录其真实类型与地址恢复能力。
    pub fn new<T>(value: T) -> Self
    where
        T: Injectable,
    {
        Self {
            service_type: ServiceType::create::<T>(),
            value: Box::new(value),
            address_resolver: &TypedAddressResolver::<T>::SHARED,
        }
    }

    /// 返回被擦除值的准确类型身份，供计划和投影核对。
    pub fn service_type(&self) -> ServiceType {
        self.service_type
    }

    /// 消费尚未共享的 envelope 并恢复其 concrete 类型。
    pub fn downcast<T>(self) -> Result<T, Self>
    where
        T: Injectable,
    {
        let Self {
            service_type,
            value,
            address_resolver,
        } = self;
        match value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => Err(Self {
                service_type,
                value,
                address_resolver,
            }),
        }
    }

    /// 从当前值的共享借用派生指针，而非复用 envelope 移动之前的借用权限。
    ///
    /// 生产调用由 DependencyLease 提供已固定在 `Arc<InstanceRecord>` 内的 envelope。
    /// 指针被使用期间不能移动或独占借用服务 Box；最后 lease 才允许移走并释放载荷。
    pub(crate) fn pointer<T>(&self) -> Option<NonNull<T>>
    where
        T: Injectable + ?Sized,
    {
        let resolver = self
            .address_resolver
            .downcast_ref::<TypedAddressResolver<T>>()?;
        (resolver.0)(&self.value)
    }
}

/// 输入准备函数使用的稳定地址凭证；即使被截留，也会保活原实例。
#[doc(hidden)]
#[derive(Clone)]
pub struct ErasedServiceRef {
    /// 保活已选真实实例的强所有权凭证。
    lease: DependencyLease,
}

impl ErasedServiceRef {
    /// 以真实实例 lease 建立可交付的擦除引用。
    pub(crate) fn new(lease: DependencyLease) -> Self {
        Self { lease }
    }

    /// 只有准确类型匹配才同时交付地址与 lease；否则返回实际类型。
    pub(crate) fn cast<T>(self) -> Result<(NonNull<T>, DependencyLease), ServiceType>
    where
        T: Injectable + ?Sized,
    {
        // 指针与 lease 必须一起交付。调用者不能在准确类型检查失败后获得裸地址。
        match self.lease.pointer::<T>() {
            Some(pointer) => Ok((pointer, self.lease)),
            None => Err(self.lease.service_type()),
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/activation/erased_service.rs"]
mod tests;
