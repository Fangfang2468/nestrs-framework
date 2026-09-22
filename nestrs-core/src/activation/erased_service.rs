//! 已构造服务的 type-erased 表示。
//!
//! 本模块定义 provider 构造结果的 owning envelope，以及构造 adapter 消费的、由未来
//! 容器提供的已验证稳定地址。它们不决定实例如何存储、共享或释放。

use std::{any::Any, ptr::NonNull};

use crate::service::{Injectable, ServiceType};

/// 由构造 adapter 返回的 owning type-erased service。
type AnyService = Box<dyn Any + Send + Sync>;

/// 已构造服务的 owning type-erased envelope。
pub struct ErasedService {
    service_type: ServiceType,
    value: AnyService,
}

impl ErasedService {
    /// 擦除一个成功构造的 concrete service。
    pub fn new<T>(value: T) -> Self
    where
        T: Injectable,
    {
        Self {
            service_type: ServiceType::create::<T>(),
            value: Box::new(value),
        }
    }

    /// 返回实际持有的 concrete service 类型。
    pub fn service_type(&self) -> ServiceType {
        self.service_type
    }

    /// 消费 type-erased service 并恢复其 concrete 类型。
    pub fn downcast<T>(self) -> Result<T, Self>
    where
        T: Injectable,
    {
        let Self {
            service_type,
            value,
        } = self;

        match value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => Err(Self {
                service_type,
                value,
            }),
        }
    }
}

/// 宏构造 adapter 使用的、已验证的具体服务稳定地址。
///
/// 此类型只在未来容器实现与宏生成的 `prepare_*` adapter 之间传递。它不拥有服务、
/// 不提供解引用或任意类型转换入口，也不表达实例存储或生命周期模型。
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ErasedServiceRef {
    pointer: NonNull<u8>,
    service_type: ServiceType,
}

impl ErasedServiceRef {
    /// 从未来容器已经验证的稳定擦除地址创建构造期引用。
    ///
    /// # Safety
    ///
    /// `pointer` 必须指向由容器持有的、实际类型与 `service_type` 一致的服务实例。
    /// 容器必须在所有由此引用构造出的注入 token 被销毁前保持该地址有效、稳定且
    /// 不被替换或单独释放。
    #[allow(dead_code)] // activation runtime 接线后由其创建。
    pub(crate) unsafe fn from_stable_erased_parts(
        pointer: NonNull<u8>,
        service_type: ServiceType,
    ) -> Self {
        Self {
            pointer,
            service_type,
        }
    }

    /// 将已验证的擦除地址恢复为宏调用点保留的准确 concrete 类型。
    ///
    /// `T` 来自 `prepare_required::<T>` 等单态化 adapter，而不是由运行时根据
    /// `TypeId` 猜测出来。
    pub(crate) fn cast<T>(self) -> Result<NonNull<T>, ServiceType>
    where
        T: Injectable,
    {
        if self.service_type != ServiceType::create::<T>() {
            return Err(self.service_type);
        }

        Ok(self.pointer.cast())
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use super::ErasedServiceRef;
    use crate::service::ServiceType;

    #[test]
    fn erased_service_ref_validates_the_concrete_type_before_casting() {
        let mut value = 7_u32;
        let reference = unsafe {
            ErasedServiceRef::from_stable_erased_parts(
                NonNull::from(&mut value).cast(),
                ServiceType::create::<u32>(),
            )
        };

        assert_eq!(
            reference
                .cast::<u32>()
                .expect("matching concrete type should cast"),
            NonNull::from(&mut value)
        );
        assert!(matches!(
            reference.cast::<u64>(),
            Err(actual) if actual == ServiceType::create::<u32>()
        ));
    }
}
