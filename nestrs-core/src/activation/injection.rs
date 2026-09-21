//! 宏生成字段使用的非 owning 注入 token ABI。
//!
//! `Injection<T>` 只保存容器已经验证过的稳定服务地址：它不拥有 `T`、不参与引用
//! 计数、不能构造新实例，也不能按类型继续解析服务。实际实例必须由未来的 root、scope
//! 或消费者 `InstanceRecord` 持有，并在所有消费者销毁后才释放。

use std::{marker::PhantomData, ops::Deref, ptr::NonNull};

use crate::service::Injectable;

/// 宏 ABI 的只读字段注入 token。
///
/// 一个 `Injection<T>` 表示“此消费者已获得由容器拥有的 `T`”。它只能解引用为 `&T`；
/// 不提供 `Clone`、`Copy`、可变访问、裸指针导出或公开构造入口。
///
/// `T` 可以是 `dyn Trait`。这时 `ptr` 是由 bind 的 typed projector 创建的完整 trait
/// object 指针；服务实例的 owner 仍然是对应的 concrete `InstanceRecord`。
pub struct Injection<T: ?Sized> {
    ptr: NonNull<T>,
    // token 不拥有 T，也不应把 T 的 pin/drop 语义带到自身；它仅保留正确的类型变型。
    _marker: PhantomData<fn() -> T>,
}

impl<T: ?Sized> Injection<T>
where
    T: Injectable,
{
    /// 从容器已经验证的稳定服务地址创建字段 token。
    ///
    /// # Safety
    ///
    /// `ptr` 必须指向精确的 `T`。创建后，实例 owner 必须在所有保存该 token 的消费者
    /// 被 cleanup/drop 前保持 `ptr` 有效；不得替换或单独释放该实例。
    pub(crate) unsafe fn from_service_ptr(ptr: NonNull<T>) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// 仅供 core 将字段 token 转换为本次 factory activation 的短期借用。
    pub(crate) fn into_ptr(self) -> NonNull<T> {
        self.ptr
    }
}

impl<T: ?Sized> Deref for Injection<T>
where
    T: Injectable,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: token 只能由隐藏的 construction ABI 基于稳定 owner 创建。
        unsafe { self.ptr.as_ref() }
    }
}

// NonNull 本身不自动跨线程；这里的显式实现仅对框架允许的 Injectable 服务开放。
// Injectable 要求 Send + Sync + 'static，因此跨线程传递 token 时只能得到线程安全的
// 共享引用，且不改变服务实例的 owner 或其生命周期。
unsafe impl<T: ?Sized + Injectable> Send for Injection<T> {}
unsafe impl<T: ?Sized + Injectable> Sync for Injection<T> {}

#[cfg(test)]
mod tests {
    use super::Injection;

    struct Service;
    trait Port: Send + Sync {}

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn injection_is_send_and_sync_for_an_injectable_service() {
        assert_send_sync::<Injection<Service>>();
        assert_send_sync::<Injection<dyn Port>>();
    }
}
