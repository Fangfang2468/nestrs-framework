//! 宏生成字段使用的只读注入 token；内部 lease 保活稳定实例及其依赖。

use std::{ops::Deref, ptr::NonNull};

use super::DependencyLease;
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
    _lease: DependencyLease,
}

impl<T: ?Sized> Injection<T>
where
    T: Injectable,
{
    /// 从容器已经验证的稳定服务地址创建字段 token。
    ///
    /// # Safety
    ///
    /// `ptr` 必须指向 `lease` 保活的实例或通过有效 typed projector 得到的子视图。
    /// 地址必须在该实例存活期间保持有效、不可变且不被替换。
    pub(crate) unsafe fn from_service_ptr(ptr: NonNull<T>, lease: DependencyLease) -> Self {
        Self { ptr, _lease: lease }
    }

    /// 仅供已独立保留同一实例 lease 的 frame 或 owner journal 取得地址。
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
        // SAFETY: the token retains its own lease to the exact projected allocation.
        unsafe { self.ptr.as_ref() }
    }
}

// Keep the same bounds as Injectable's blanket impl, but spell out the auto traits here.
// Routing these obligations through a custom trait prevents coinductive Send/Sync checking for
// mutually injected class types, rejecting them before graph compilation can diagnose the cycle.
// The retained lease makes the immutable pointer valid for every use of this thread-safe token.
unsafe impl<T: ?Sized + Send + Sync + 'static> Send for Injection<T> {}
unsafe impl<T: ?Sized + Send + Sync + 'static> Sync for Injection<T> {}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::Injection;
    use crate::activation::{
        DependencyLease, ErasedService, InputSlot, ReleaseDomain, prepare_required,
    };

    struct Service;
    trait Port: Send + Sync {}

    fn assert_send_sync<T: Send + Sync>() {}

    #[test]
    fn injection_is_send_and_sync_for_an_injectable_service() {
        assert_send_sync::<Injection<Service>>();
        assert_send_sync::<Injection<dyn Port>>();
    }

    #[test]
    fn a_token_moved_out_by_consumer_drop_keeps_the_dependency_alive() {
        struct Dependency {
            value: u32,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for Dependency {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
            }
        }
        struct Consumer {
            dependency: Option<Injection<Dependency>>,
            escaped: Arc<Mutex<Option<Injection<Dependency>>>>,
        }
        impl Drop for Consumer {
            fn drop(&mut self) {
                *self.escaped.lock().unwrap() = self.dependency.take();
            }
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let escaped = Arc::new(Mutex::new(None));
        let domain = ReleaseDomain::new();
        let dependency = DependencyLease::new(
            ErasedService::new(Dependency {
                value: 42,
                drops: drops.clone(),
            }),
            vec![],
            domain.clone(),
        );
        let token =
            prepare_required::<Dependency>(InputSlot::new(0), Some(dependency.erased_ref()))
                .unwrap()
                .into_required(InputSlot::new(0))
                .unwrap();
        let consumer = DependencyLease::new(
            ErasedService::new(Consumer {
                dependency: Some(token),
                escaped: escaped.clone(),
            }),
            vec![dependency],
            domain,
        );
        drop(consumer);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let token = escaped.lock().unwrap().take().unwrap();
        assert_eq!(token.value, 42);
        drop(token);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}
