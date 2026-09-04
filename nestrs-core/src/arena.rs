//! 单次实例化所使用的稳定地址服务存储。
//!
//! [`Arena`] 不构建依赖图，也不解释生命周期配置。它只接收已经成功构造的具体服务，
//! 将其移动到不会移动的原始存储中，并按提交的反向顺序析构。宏生成的
//! [`crate::inject_wrapper::Inject`] 令牌因此可以只保存已验证的稳定指针。

use std::{
    alloc::{Layout, alloc, dealloc},
    collections::{HashMap, HashSet},
    num::NonZeroUsize,
    ptr::NonNull,
};

use thiserror::Error;

use crate::{
    construction::{ErasedDropFn, ErasedService},
    registration::{
        injectable::Injectable, service_identifier::ServiceIdentifier, service_type::ServiceType,
    },
};

/// 已提交服务的类型擦除稳定地址。
///
/// 该值只能由 [`Arena::lookup`] 产生，并仅供宏生成的输入准备 ABI 使用；它不提供
/// 解引用、所有权转移或任意类型转换入口。
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct ArenaServiceRef {
    pointer: NonNull<u8>,
    service_type: ServiceType,
}

impl ArenaServiceRef {
    /// 将已验证的擦除地址恢复为精确 concrete 类型。
    ///
    /// `T` 来自宏输出的 `prepare_required::<T>` / `prepare_optional::<T>`，而不是
    /// runtime 从 `TypeId` 猜测出来的类型。
    pub(crate) fn cast<T>(
        self,
        position: crate::construction::InputPosition,
    ) -> Result<NonNull<T>, crate::construction::ActivationError>
    where
        T: Injectable,
    {
        if self.service_type != ServiceType::create::<T>() {
            return Err(crate::construction::ActivationError::InputTypeMismatch {
                position,
                expected: std::any::type_name::<T>(),
                actual: self.service_type.name,
            });
        }

        Ok(self.pointer.cast())
    }
}

/// `Arena` 操作失败时返回的受控错误。
#[derive(Debug, Error)]
pub enum ArenaError {
    #[error("服务 {identifier:?} 尚未实例化")]
    MissingService { identifier: ServiceIdentifier },

    #[error("服务 {identifier:?} 已经实例化，Arena 不允许替换已发布实例")]
    ServiceAlreadyCommitted { identifier: ServiceIdentifier },

    #[error("准备提交 {identifier:?} 时，构造结果类型应为 {expected:?}，实际为 {actual:?}")]
    ServiceTypeMismatch {
        identifier: ServiceIdentifier,
        expected: ServiceType,
        actual: ServiceType,
    },

    #[error("Arena 无法为服务索引预留元数据")]
    MetadataAllocationFailed,

    #[error("分配器拒绝了大小为 {size}、对齐为 {align} 的服务存储")]
    AllocationFailed { size: usize, align: usize },
}

/// 按提交顺序保存的原始分配；它只负责释放内存，不负责运行用户析构函数。
struct RawAllocation {
    pointer: NonNull<u8>,
    layout: Layout,
}

/// 一个 service 的类型化析构任务。
struct DropEntry {
    pointer: NonNull<u8>,
    drop_value: ErasedDropFn,
}

/// 当前实例化会话的服务 Arena。
///
/// 一个 `Arena` 中每个 [`ServiceIdentifier`] 最多发布一次。后续服务可通过
/// `Inject<T>` 指向先前提交的依赖；因此没有单项删除或替换 API。销毁 Arena 时，
/// 消费者会先于其依赖逆序析构。
pub struct Arena {
    allocations: Vec<RawAllocation>,
    services: HashMap<ServiceIdentifier, ArenaServiceRef>,
    drop_log: Vec<DropEntry>,
    building: HashSet<ServiceIdentifier>,
}

impl Arena {
    /// 创建空的单次实例化 Arena。
    pub fn new() -> Self {
        Self {
            allocations: Vec::new(),
            services: HashMap::new(),
            drop_log: Vec::new(),
            building: HashSet::new(),
        }
    }

    /// 判断指定精确服务身份是否已发布到当前 Arena。
    pub fn contains(&self, identifier: ServiceIdentifier) -> bool {
        self.services.contains_key(&identifier)
    }

    /// 读取无 key 的具体服务。
    ///
    /// 返回的引用由 Arena 借用约束，不能比 Arena 活得更久。
    pub fn get<T>(&self) -> Result<&T, ArenaError>
    where
        T: Injectable,
    {
        self.get_by_identifier(ServiceIdentifier::from(ServiceType::create::<T>()))
    }

    /// 读取精确服务身份对应的具体服务。
    pub fn get_by_identifier<T>(&self, identifier: ServiceIdentifier) -> Result<&T, ArenaError>
    where
        T: Injectable,
    {
        let service = self
            .lookup(identifier)
            .ok_or(ArenaError::MissingService { identifier })?;
        let pointer = service
            .cast::<T>(crate::construction::InputPosition(0))
            .map_err(|_| ArenaError::ServiceTypeMismatch {
                identifier,
                expected: ServiceType::create::<T>(),
                actual: service.service_type,
            })?;

        // SAFETY: `lookup` only returns a service committed with the exact type proof;
        // `Arena` never moves or removes its raw allocation before this borrow ends.
        Ok(unsafe { pointer.as_ref() })
    }

    /// 手工向 Arena 提交一个 concrete 根实例。
    ///
    /// 该入口适合没有 `#[injectable]` 构造器的外部根对象；正常服务构造应通过
    /// `ServiceCollection` 与宏生成 constructor 完成。
    pub fn insert<T>(&mut self, identifier: ServiceIdentifier, value: T) -> Result<(), ArenaError>
    where
        T: Injectable,
    {
        self.commit(identifier, ErasedService::new(value))
    }

    /// 返回供 core 内部字段输入 ABI 消费的类型擦除服务引用。
    ///
    /// 此入口不能向下游 crate 公开：否则调用方可以先准备一个 `Inject<T>`，再丢弃
    /// Arena，从安全代码中制造悬垂指针。宏只在 metadata 中保存 `prepare_input` 函数项，
    /// 实际查找与调用始终由本 crate 的实例化 runtime 完成。
    pub(crate) fn lookup(&self, identifier: ServiceIdentifier) -> Option<ArenaServiceRef> {
        self.services.get(&identifier).copied()
    }

    /// 标记服务正在构造，用于阻止递归激活无限重入。
    ///
    /// 这不是依赖图构建或图验证；它只保护单次按需递归实例化不会因运行时重入耗尽栈。
    pub(crate) fn begin_building(&mut self, identifier: ServiceIdentifier) -> bool {
        self.building.insert(identifier)
    }

    /// 在一次构造尝试结束后清除运行期重入标记。
    pub(crate) fn finish_building(&mut self, identifier: ServiceIdentifier) {
        self.building.remove(&identifier);
    }

    /// 将宏构造 adapter 返回的具体服务移动至稳定 Arena 地址并发布其身份。
    pub(crate) fn commit(
        &mut self,
        identifier: ServiceIdentifier,
        service: ErasedService,
    ) -> Result<(), ArenaError> {
        if self.services.contains_key(&identifier) {
            return Err(ArenaError::ServiceAlreadyCommitted { identifier });
        }

        let actual = service.service_type();
        let expected = identifier.service_type;
        if actual != expected {
            return Err(ArenaError::ServiceTypeMismatch {
                identifier,
                expected,
                actual,
            });
        }

        self.services
            .try_reserve(1)
            .map_err(|_| ArenaError::MetadataAllocationFailed)?;
        self.drop_log
            .try_reserve(1)
            .map_err(|_| ArenaError::MetadataAllocationFailed)?;

        let pointer = self.allocate(service.layout())?;
        let drop_value = service.drop_value();

        // SAFETY: `allocate` returned fresh storage with the exact layout reported by this
        // `ErasedService`. The service is consumed exactly once and only published afterwards.
        unsafe { service.transfer_into(pointer) };

        let old = self.services.insert(
            identifier,
            ArenaServiceRef {
                pointer,
                service_type: actual,
            },
        );
        debug_assert!(old.is_none());
        self.drop_log.push(DropEntry {
            pointer,
            drop_value,
        });
        Ok(())
    }

    fn allocate(&mut self, layout: Layout) -> Result<NonNull<u8>, ArenaError> {
        if layout.size() == 0 {
            return Ok(Self::aligned_zst_pointer(layout.align()));
        }

        self.allocations
            .try_reserve(1)
            .map_err(|_| ArenaError::MetadataAllocationFailed)?;

        // SAFETY: `layout` comes from `Layout::new::<T>()` in `ErasedService` and is valid.
        let pointer = unsafe { alloc(layout) };
        let pointer = NonNull::new(pointer).ok_or(ArenaError::AllocationFailed {
            size: layout.size(),
            align: layout.align(),
        })?;
        self.allocations.push(RawAllocation { pointer, layout });
        Ok(pointer)
    }

    fn aligned_zst_pointer(align: usize) -> NonNull<u8> {
        let address = NonZeroUsize::new(align)
            .expect("Layout alignment must always be non-zero for a zero-sized type");
        NonNull::without_provenance(address)
    }

    fn clear_allocations(&mut self) {
        while let Some(allocation) = self.allocations.pop() {
            // SAFETY: every non-zero allocation is stored with its original layout and is
            // deallocated exactly once after the typed drop log has been drained.
            unsafe { dealloc(allocation.pointer.as_ptr(), allocation.layout) };
        }
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Arena {
    fn drop(&mut self) {
        while let Some(entry) = self.drop_log.pop() {
            // SAFETY: `commit` initialized this exact address once and registered the matching
            // monomorphized drop function. Popping ensures reverse order and exactly-once drop.
            unsafe { (entry.drop_value)(entry.pointer.as_ptr()) };
        }
        self.clear_allocations();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use super::*;

    static DROP_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();

    struct First;
    struct Second;

    impl Drop for First {
        fn drop(&mut self) {
            DROP_ORDER
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("drop order mutex should not be poisoned")
                .push("first");
        }
    }

    impl Drop for Second {
        fn drop(&mut self) {
            DROP_ORDER
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("drop order mutex should not be poisoned")
                .push("second");
        }
    }

    #[test]
    fn drops_consumers_before_their_already_committed_dependencies() {
        let drops = DROP_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        drops
            .lock()
            .expect("drop order mutex should not be poisoned")
            .clear();

        {
            let mut arena = Arena::new();
            arena
                .insert(
                    ServiceIdentifier::from(ServiceType::create::<First>()),
                    First,
                )
                .expect("first service should commit");
            arena
                .insert(
                    ServiceIdentifier::from(ServiceType::create::<Second>()),
                    Second,
                )
                .expect("second service should commit");
        }

        assert_eq!(
            *drops
                .lock()
                .expect("drop order mutex should not be poisoned"),
            ["second", "first"]
        );
    }

    #[test]
    fn keeps_a_committed_service_at_a_stable_address() {
        let mut arena = Arena::new();
        let identifier = ServiceIdentifier::from(ServiceType::create::<String>());
        arena
            .insert(identifier, String::from("repository"))
            .expect("service should commit");

        let first = arena
            .get_by_identifier::<String>(identifier)
            .expect("service should be readable") as *const String;

        for index in 0..64_usize {
            arena
                .insert(
                    ServiceIdentifier::new(
                        Some(crate::registration::service_key::ServiceKey::Indexed(index)),
                        ServiceType::create::<(usize, usize)>(),
                    ),
                    (index, index),
                )
                .expect("a distinct keyed service should commit");
        }

        let second = arena
            .get_by_identifier::<String>(identifier)
            .expect("service should remain readable") as *const String;
        assert_eq!(first, second);
    }
}
