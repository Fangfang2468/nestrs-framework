//! 编译器生成字段使用的只读注入令牌；内部 lease 保活稳定实例及其依赖。
//!
//! 令牌的内存所有权与 scope 的逻辑关闭分开：即使消费者在 Drop 中移出某个可选字段，
//! 逃逸令牌也继续持有它所指向的实例。它只保证内存有效，不承诺 cleanup 后业务资源
//! 仍可正常使用。关闭流程无需等待令牌归还。

use std::{ops::Deref, ptr::NonNull};

use super::DependencyLease;
use crate::service::Injectable;

/// 服务字段持有的只读注入令牌。
///
/// 一个 `Injection<T>` 表示“此消费者已获得由容器拥有的 `T`”。它只能解引用为 `&T`；
/// 不提供 `Clone`、`Copy`、可变访问、裸指针导出或公开构造入口。
///
/// `T` 可以是 `dyn Trait`。这时 `ptr` 是由编译器生成的类型化投影创建的完整 trait
/// object 指针；服务实例的 owner 仍然是对应具体类型的 `InstanceRecord`。
pub struct Injection<T: ?Sized> {
    ptr: NonNull<T>,
    _lease: DependencyLease,
}

impl<T: ?Sized> Injection<T>
where
    T: Injectable,
{
    /// 从容器已经验证的稳定服务地址创建字段令牌。
    ///
    /// # Safety
    ///
    /// `ptr` 必须指向 `lease` 保活的实例或通过有效类型化投影得到的子视图。
    /// 地址必须在该实例存活期间保持有效、不可变且不被替换。
    pub(crate) unsafe fn from_service_ptr(ptr: NonNull<T>, lease: DependencyLease) -> Self {
        Self { ptr, _lease: lease }
    }

    /// 消费令牌并取出地址，仅供已经独立持有同一实例 lease 的工厂帧或 owner 使用。
    ///
    /// 本方法会释放令牌自己的 lease，返回的 `NonNull` 本身不延长实例存活期。
    /// 地址解引用处必须确认工厂帧或 owner 记录仍持有准确实例。
    pub(crate) fn into_ptr(self) -> NonNull<T> {
        self.ptr
    }

    pub(crate) fn lease(&self) -> DependencyLease {
        self._lease.clone()
    }
}

impl<T: ?Sized> Deref for Injection<T>
where
    T: Injectable,
{
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: 本令牌持有准确投影实例的强 lease；字段构造保证地址稳定且只读，
        // 返回引用的存活期又被当前令牌借用限制，因此解引用期间实例不可能释放。
        unsafe { self.ptr.as_ref() }
    }
}

// SAFETY: 令牌持有准确实例的强 lease，且 T 满足跨线程移动和共享不可变读取的要求。
// 这里直接写出 Injectable blanket impl 的 auto trait 约束，不能间接写成 T: Injectable；
// 后者会阻断相互注入类型的 Send/Sync 共归纳检查，使 Rust 提前拒绝类型，无法交由图
// 编译器给出真正的循环依赖诊断。
unsafe impl<T: ?Sized + Send + Sync + 'static> Send for Injection<T> {}
// SAFETY: 与上面的 Send 相同，共享令牌只会得到共享引用，lease 覆盖全部读取的存活期。
unsafe impl<T: ?Sized + Send + Sync + 'static> Sync for Injection<T> {}

#[cfg(test)]
#[path = "../../tests/unit/activation/injection.rs"]
mod tests;
