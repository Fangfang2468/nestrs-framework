//! 已构造服务的 owning envelope 与带 lease 的构造期引用。

use std::{any::Any, ptr::NonNull};

use super::DependencyLease;
use crate::service::{Injectable, ServiceType};

type AnyService = Box<dyn Any + Send + Sync>;

/// 仅保存准确类型的稳定地址，不提供独立解引用入口。
struct TypedAddress<T: ?Sized>(NonNull<T>);

// The address is only dereferenced while its immutable boxed service is retained by a lease.
unsafe impl<T: Injectable + ?Sized> Send for TypedAddress<T> {}
unsafe impl<T: Injectable + ?Sized> Sync for TypedAddress<T> {}

/// 已构造服务的 owning type-erased envelope。
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

/// 宏 preparer 使用的稳定地址凭证；即使被截留，也会保活原实例。
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
        T: Injectable,
    {
        match self.lease.pointer::<T>() {
            Some(pointer) => Ok((pointer, self.lease)),
            None => Err(self.lease.service_type()),
        }
    }
}

#[cfg(test)]
mod tests {
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
        // SAFETY: retained still owns the allocation.
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
}
