//! 已构造服务的类型擦除容器与带 lease 的构造期引用。
//!
//! 运行时可以统一存放不同服务，但不能因此丢失准确类型信息。服务值与 typed address
//! 分别放进 Box：前者拥有实例，后者记录它的准确 `NonNull<T>` 类型。移动这个容器不
//! 会移动服务；恢复指针时仍通过 `Any` 检查完整类型，不能只比较字符串名称。

use std::{any::Any, ptr::NonNull};

use super::DependencyLease;
use crate::service::{Injectable, ServiceType};

type AnyService = Box<dyn Any + Send + Sync>;

/// 仅保存准确类型的稳定地址，不提供独立解引用入口。
struct TypedAddress<T: ?Sized>(NonNull<T>);

// SAFETY: T 满足 Injectable 的 Send + Sync 约束。此类型只传递地址，不提供解引用；
// 实际读取必须持有对应不可变 Box 实例的 lease，跨线程传递不会提前释放实例。
unsafe impl<T: Injectable + ?Sized> Send for TypedAddress<T> {}
// SAFETY: 所有读取都是共享不可变访问，且每次读取的存活期都由对应实例的 lease 覆盖。
unsafe impl<T: Injectable + ?Sized> Sync for TypedAddress<T> {}

/// 拥有服务值的类型擦除容器；发布后由实例记录持有，不能再移出其中的值。
pub struct ErasedService {
    service_type: ServiceType,
    value: AnyService,
    address: AnyService,
}

impl ErasedService {
    /// 擦除一个成功构造的 concrete service，并固定其地址。
    pub fn new<T>(value: T) -> Self
    where
        T: Injectable,
    {
        let value = Box::new(value);
        let address = Box::new(TypedAddress(NonNull::from(value.as_ref())));
        Self {
            service_type: ServiceType::create::<T>(),
            value,
            address,
        }
    }

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
            address,
        } = self;
        match value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => Err(Self {
                service_type,
                value,
                address,
            }),
        }
    }

    pub(crate) fn pointer<T>(&self) -> Option<NonNull<T>>
    where
        T: Injectable + ?Sized,
    {
        self.address
            .downcast_ref::<TypedAddress<T>>()
            .map(|address| address.0)
    }
}

/// 输入准备函数使用的稳定地址凭证；即使被截留，也会保活原实例。
#[doc(hidden)]
#[derive(Clone)]
pub struct ErasedServiceRef {
    lease: DependencyLease,
}

impl ErasedServiceRef {
    pub(crate) fn new(lease: DependencyLease) -> Self {
        Self { lease }
    }

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
