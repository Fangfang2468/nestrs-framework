//! 宏构造 adapter 使用的稳定服务引用 ABI。
//!
//! 这里刻意不实现服务存储、生命周期、Scope 或调度。未来的容器实现只要能在调用
//! construction adapter 期间保证地址有效，就可以构造这个内部 token。

use std::ptr::NonNull;

use crate::{
    construction::{ActivationError, InputPosition},
    registration::{injectable::Injectable, service_type::ServiceType},
};

/// 已验证的具体服务稳定地址。
///
/// 这个类型只在宏生成的 prepare_* adapter 与未来的容器实现之间传递；它不提供
/// 解引用、所有权转移或任意类型转换入口。
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ArenaServiceRef {
    pointer: NonNull<u8>,
    service_type: ServiceType,
}

impl ArenaServiceRef {
    /// 将已验证的擦除地址恢复为宏调用点保留的准确 concrete 类型。
    ///
    /// T 来自 prepare_required::<T> 等单态化 adapter，而不是由运行时根据
    /// TypeId 猜测出来。
    pub(crate) fn cast<T>(self, position: InputPosition) -> Result<NonNull<T>, ActivationError>
    where
        T: Injectable,
    {
        if self.service_type != ServiceType::create::<T>() {
            return Err(ActivationError::InputTypeMismatch {
                position,
                expected: std::any::type_name::<T>(),
                actual: self.service_type.name,
            });
        }

        Ok(self.pointer.cast())
    }
}
