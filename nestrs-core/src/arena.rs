//! 单次实例化所使用的稳定地址服务存储。
//!
//! [`Arena`] 不构建依赖图，也不解释生命周期配置。它只接收已经成功构造的具体服务，
//! 将其保存在不会移动的 Box 中，并按提交的反向顺序析构。宏生成的
//! [`crate::inject_wrapper::Inject`] 令牌因此可以只保存已验证的稳定指针。

use std::{
    collections::HashMap,
    future::poll_fn,
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Poll, Waker},
};

use thiserror::Error;

use crate::{
    construction::ErasedService,
    registration::{
        injectable::Injectable, provider::CleanupHook, service_identifier::ServiceIdentifier,
        service_type::ServiceType,
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

    /// 保留既有错误契约；v6 的服务值由 `Box` 分配，正常提交路径不再手动返回此变体。
    #[error("分配器拒绝了大小为 {size}、对齐为 {align} 的服务存储")]
    AllocationFailed { size: usize, align: usize },
}

/// 一个 service 的类型化析构任务。
struct DropEntry {
    identifier: ServiceIdentifier,
    /// `ErasedService` 内部的 Box 是实际 service 的稳定存储。
    ///
    /// journal 自身可以在 Vec 扩容时移动，但这个 Box 的 data pointer 不会移动；所有
    /// `ArenaServiceRef` 和由它准备的 `Inject<T>` 都因此保持有效，直至 entry 被销毁。
    service: ErasedService,
    cleanup: Option<CleanupHook>,
    /// 由此服务直接拥有的 transient 子树。
    ///
    /// 每个 child 是独立的 Arena，因而不会和父 Arena 的按 token 索引冲突；并且 child
    /// 只会在父服务值已经析构后才被销毁或 shutdown。这保证字段中的 `Inject<T>` 在父值
    /// 存活期间始终指向有效的 transient 实例。
    children: Vec<ArenaOwner>,
}

impl DropEntry {
    fn service_ref(&self) -> ArenaServiceRef {
        ArenaServiceRef {
            pointer: self.service.stable_pointer(),
            service_type: self.service.service_type(),
        }
    }

    /// 普通 rollback / Drop 的析构路径。它绝不调用 cleanup hook，并明确保持
    /// `consumer value -> transient children` 顺序。
    fn drop_without_cleanup(self) {
        let Self {
            identifier: _,
            service,
            cleanup: _,
            children,
        } = self;
        drop(service);
        drop(children);
    }
}

/// 一个 Arena backing 的可跨线程保活令牌。
///
/// 它不提供查询、提交或析构入口；唯一职责是让 worker 持有的 `ConstructionContext`
/// 中的 `Inject<T>` 地址在 task 结束前继续有效。令牌不借用 [`Arena`]，因此可以进入
/// Tokio 的 `'static` worker future。
pub(crate) struct ArenaLease {
    backing: Arc<ArenaBacking>,
}

impl Clone for ArenaLease {
    fn clone(&self) -> Self {
        Self::new(Arc::clone(&self.backing))
    }
}

impl ArenaLease {
    fn new(backing: Arc<ArenaBacking>) -> Self {
        backing.acquire_lease();
        Self { backing }
    }
}

impl Drop for ArenaLease {
    fn drop(&mut self) {
        self.backing.release_lease();
    }
}

/// 一个 worker 可见 Arena 闭包的 owning lease 集合。
///
/// `ArenaLeaseSet` 不保留 `&Arena`，且其中的 backing 只包含 `Send + Sync` 的服务值和
/// 同步 journal 元数据，因此它本身可安全地移动进 Tokio worker。调度器应在每次把
/// prepared activation 移入 worker 前，覆盖所有可能由其 `ConstructionContext` 解引用
/// 的 Singleton、Scoped 与 transient source Arena。lease 按外到内加入，并必须按内到
/// 外释放：这会使已取消 worker 的当前/transient Arena 先完成普通 Rust Drop，最后才
/// 允许祖先 scope 或 Singleton Arena 的 `shutdown()` 越过 lease barrier。
#[derive(Default)]
pub(crate) struct ArenaLeaseSet {
    leases: Vec<ArenaLease>,
}

impl ArenaLeaseSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 从一组短借用的 Arena 创建不再借用它们的 `'static` lease set。
    pub(crate) fn from_arenas<'arena>(arenas: impl IntoIterator<Item = &'arena Arena>) -> Self {
        let mut leases = Self::new();
        for arena in arenas {
            leases.push_arena(arena);
        }
        leases
    }

    pub(crate) fn push(&mut self, lease: ArenaLease) {
        self.leases.push(lease);
    }

    pub(crate) fn push_arena(&mut self, arena: &Arena) {
        self.push(arena.lease());
    }
}

impl Drop for ArenaLeaseSet {
    fn drop(&mut self) {
        // `Vec` 的自动 element-drop 顺序不是这个 lifecycle contract 的合适表达。
        // A worker leases sources from parent to child; releasing with `pop` makes every
        // consumer/transient backing quiesce before its ancestor may start shutdown.
        while let Some(lease) = self.leases.pop() {
            drop(lease);
        }
    }
}

/// 由 Arena facade、transient child journal 和 worker lease 共同持有的 backing。
///
/// `Mutex` 只保护 coordinator 的短暂提交/查询边界与 shutdown 从 journal 中取走 entry
/// 的动作，绝不跨用户 future 的 await 保持。service 访问本身仍走稳定的 `Inject<T>`
/// pointer；worker 不会通过此类型查询或修改 Arena。
struct ArenaBacking {
    state: Mutex<ArenaState>,
    active_leases: AtomicUsize,
    lease_waiters: Mutex<Vec<Waker>>,
}

#[derive(Default)]
struct ArenaState {
    /// exact token -> journal slot。slot 指向 `DropEntry::service` 的 stable Box，而不是
    /// 可跨线程传播的 raw allocation。
    services: HashMap<ServiceIdentifier, usize>,
    drop_log: Vec<DropEntry>,
}

impl ArenaBacking {
    fn new() -> Self {
        Self {
            state: Mutex::new(ArenaState::default()),
            active_leases: AtomicUsize::new(0),
            lease_waiters: Mutex::new(Vec::new()),
        }
    }

    fn state(&self) -> MutexGuard<'_, ArenaState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn acquire_lease(&self) {
        self.active_leases
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .expect("Arena lease counter must not overflow");
    }

    fn release_lease(&self) {
        let previous = self
            .active_leases
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_sub(1)
            })
            .expect("Arena lease counter must not underflow");
        if previous != 1 {
            return;
        }

        let waiters = {
            let mut waiters = self
                .lease_waiters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *waiters)
        };
        for waiter in waiters {
            waiter.wake();
        }
    }

    async fn wait_for_leases(&self) {
        poll_fn(|context| {
            if self.active_leases.load(Ordering::Acquire) == 0 {
                return Poll::Ready(());
            }

            let mut waiters = self
                .lease_waiters
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            // Recheck under the waiter lock so a last lease cannot disappear between the first
            // observation and waker registration.
            if self.active_leases.load(Ordering::Acquire) == 0 {
                return Poll::Ready(());
            }
            if !waiters
                .iter()
                .any(|registered| registered.will_wake(context.waker()))
            {
                waiters.push(context.waker().clone());
            }
            Poll::Pending
        })
        .await;
    }

    async fn shutdown_contents(&self) {
        loop {
            let entry = {
                let mut state = self.state();
                let Some(entry) = state.drop_log.pop() else {
                    break;
                };
                let removed = state.services.remove(&entry.identifier);
                debug_assert!(
                    removed.is_some(),
                    "each journal entry must have an index slot"
                );
                entry
            };

            if let Some(cleanup) = entry.cleanup {
                cleanup().await;
            }

            let DropEntry {
                identifier: _,
                service,
                cleanup: _,
                mut children,
            } = entry;
            // The consumer's service value must die before its field-owned transient child
            // arenas. If this shutdown future is cancelled, both locals use ordinary Drop and
            // retain exactly the same order without polling any remaining hook.
            drop(service);
            while let Some(child) = children.pop() {
                // A child backing can itself contain nested transient ownership. Boxing keeps
                // the recursively composed shutdown future finite without imposing Send on the
                // public Arena facade.
                Box::pin(child.shutdown()).await;
            }
        }
    }
}

impl Drop for ArenaBacking {
    fn drop(&mut self) {
        debug_assert_eq!(
            self.active_leases.load(Ordering::Acquire),
            0,
            "a backing cannot be dropped while a lease retains its Arc"
        );
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while let Some(entry) = state.drop_log.pop() {
            state.services.remove(&entry.identifier);
            entry.drop_without_cleanup();
        }
        state.services.clear();
    }
}

/// Arena 的实际所有者。它刻意不带 `Rc` marker：child journal 与 worker lease 只能持有
/// 这个 private handle，而不能获得公开 facade 的查询/提交能力。
struct ArenaOwner {
    backing: Arc<ArenaBacking>,
}

impl ArenaOwner {
    fn new() -> Self {
        Self {
            backing: Arc::new(ArenaBacking::new()),
        }
    }

    fn lease(&self) -> ArenaLease {
        ArenaLease::new(Arc::clone(&self.backing))
    }

    async fn shutdown(self) {
        // `self` intentionally remains alive across this await. A caller that consumes an Arena
        // cannot create a new lease afterwards, while already spawned workers retain a lease
        // until their frame/context/output have been dropped.
        self.backing.wait_for_leases().await;
        self.backing.shutdown_contents().await;
    }
}

/// 当前实例化会话的服务 Arena facade。
///
/// 一个 `Arena` 中每个 [`ServiceIdentifier`] 最多发布一次。后续服务可通过
/// `Inject<T>` 指向先前提交的依赖；因此没有单项删除或替换 API。销毁 Arena 时，
/// 消费者会先于其依赖逆序析构。backing 可以由 worker lease 持有，但 facade 本身保持
/// 线程封闭，防止内部 Arc 化意外让 ServiceProvider / Scope 获得 Send 或 Sync
/// 承诺。
pub(crate) struct Arena {
    owner: ArenaOwner,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl Arena {
    /// 创建空的单次实例化 Arena。
    pub(crate) fn new() -> Self {
        Self {
            owner: ArenaOwner::new(),
            _not_send_or_sync: PhantomData,
        }
    }

    /// 判断指定精确服务身份是否已发布到当前 Arena。
    pub(crate) fn contains(&self, identifier: ServiceIdentifier) -> bool {
        self.owner
            .backing
            .state()
            .services
            .contains_key(&identifier)
    }

    /// 读取无 key 的具体服务。
    ///
    /// 仅供 core 的 `root()`/`get()` façade 读取默认 key concrete 服务。公共查询不会
    /// 触发动态解析或创建服务。
    pub(crate) fn get<T>(&self) -> Result<&T, ArenaError>
    where
        T: Injectable,
    {
        self.get_by_identifier(ServiceIdentifier::from(ServiceType::create::<T>()))
    }

    /// 读取精确服务身份对应的具体服务。
    pub(crate) fn get_by_identifier<T>(
        &self,
        identifier: ServiceIdentifier,
    ) -> Result<&T, ArenaError>
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

        // SAFETY: `lookup` only returns a service committed with the exact type proof. The
        // pointer targets the `ErasedService`'s Box allocation, which remains stable for this
        // Arena's lifetime; the facade is !Send/!Sync so safe code cannot consume it through a
        // concurrent shutdown while this shared borrow exists.
        Ok(unsafe { pointer.as_ref() })
    }

    /// 供 core 单元测试提交一个已构造的 concrete 服务。
    #[cfg(test)]
    pub(crate) fn insert<T>(
        &mut self,
        identifier: ServiceIdentifier,
        value: T,
    ) -> Result<(), ArenaError>
    where
        T: Injectable,
    {
        self.commit(identifier, ErasedService::new(value))
    }

    /// 返回供 core 内部字段输入 ABI 消费的类型擦除服务引用。
    ///
    /// 此入口不能向下游 crate 公开：否则调用方可以先准备一个 `Inject<T>`，再丢弃
    /// Arena，从安全代码中制造悬垂指针。宏只在 `Provider` 中保存 `prepare_input` 函数项，
    /// 实际查找与调用将由本 crate 的未来激活器完成。
    pub(crate) fn lookup(&self, identifier: ServiceIdentifier) -> Option<ArenaServiceRef> {
        let state = self.owner.backing.state();
        let index = *state.services.get(&identifier)?;
        let entry = state
            .drop_log
            .get(index)
            .expect("a published Arena token must point at its journal entry");
        debug_assert_eq!(entry.identifier, identifier);
        Some(entry.service_ref())
    }

    /// 创建一个仅保活本 Arena backing 的 worker lease。
    pub(crate) fn lease(&self) -> ArenaLease {
        self.owner.lease()
    }

    /// 将宏构造 adapter 返回的具体服务移动至稳定 Arena 地址并发布其身份。
    ///
    /// 该兼容入口不关联 cleanup hook。需要将 provider 声明的 hook 随服务一同记录时，
    /// core activation runtime 应使用 [`Self::commit_with_cleanup`]。
    #[allow(dead_code)] // 保留给不参与 lifecycle 的 core 内部兼容调用方。
    pub(crate) fn commit(
        &mut self,
        identifier: ServiceIdentifier,
        service: ErasedService,
    ) -> Result<(), ArenaError> {
        self.commit_with_cleanup(identifier, service, None)
    }

    /// 将一个具体服务及其可选 cleanup hook 移动至稳定 Arena 地址并发布其身份。
    ///
    /// hook 只由 [`Self::shutdown`] 调用；普通 [`Drop`] 仍只负责值析构，避免隐式启动
    /// 异步工作。hook 会与对应值按 commit 逆序处理，并在该值析构前完成。
    pub(crate) fn commit_with_cleanup(
        &mut self,
        identifier: ServiceIdentifier,
        service: ErasedService,
        cleanup: Option<CleanupHook>,
    ) -> Result<(), ArenaError> {
        self.commit_with_cleanup_and_children(identifier, service, cleanup, Vec::new())
    }

    /// 将一个具体服务、其可选 cleanup hook 与其直接拥有的 transient 子树一同提交。
    ///
    /// `children` 的所有权在提交成功后转移给该 service 的 [`DropEntry`]。普通 Arena
    /// 析构时会先析构 service 值，再让这些 child Arena 依次普通析构；显式
    /// [`Self::shutdown`] 时则先执行 service 自身 hook、析构 service 值，最后以 child
    /// 的反向移交顺序 shutdown 每个子树。若提交失败，传入 children 会自然普通析构，
    /// 因而构建失败不会意外运行异步 cleanup。
    pub(crate) fn commit_with_cleanup_and_children(
        &mut self,
        identifier: ServiceIdentifier,
        service: ErasedService,
        cleanup: Option<CleanupHook>,
        children: Vec<Arena>,
    ) -> Result<(), ArenaError> {
        let actual = service.service_type();
        let expected = identifier.service_type;
        if actual != expected {
            return Err(ArenaError::ServiceTypeMismatch {
                identifier,
                expected,
                actual,
            });
        }

        let mut owned_children = Vec::new();
        owned_children
            .try_reserve(children.len())
            .map_err(|_| ArenaError::MetadataAllocationFailed)?;
        for child in children {
            owned_children.push(child.into_owner());
        }

        let pointer = service.stable_pointer();
        let mut state = self.owner.backing.state();
        if state.services.contains_key(&identifier) {
            return Err(ArenaError::ServiceAlreadyCommitted { identifier });
        }
        state
            .services
            .try_reserve(1)
            .map_err(|_| ArenaError::MetadataAllocationFailed)?;
        state
            .drop_log
            .try_reserve(1)
            .map_err(|_| ArenaError::MetadataAllocationFailed)?;

        let slot = state.drop_log.len();
        let old = state.services.insert(identifier, slot);
        debug_assert!(old.is_none());
        state.drop_log.push(DropEntry {
            identifier,
            service,
            cleanup,
            children: owned_children,
        });
        debug_assert_eq!(
            state.drop_log[slot].service_ref().pointer,
            pointer,
            "publishing must preserve the Box data pointer used for input preparation"
        );
        Ok(())
    }

    /// 按 commit 逆序运行 cleanup hook 并析构已发布服务。
    ///
    /// shutdown 会先等待已发布的 worker lease 归零，再按 commit 逆序运行 hook、析构值
    /// 并 shutdown transient children。普通 Drop、build rollback 与 future cancellation
    /// 不会调用 hook。
    pub(crate) async fn shutdown(self) {
        let Self {
            owner,
            _not_send_or_sync: _,
        } = self;
        owner.shutdown().await;
    }

    fn into_owner(self) -> ArenaOwner {
        let Self {
            owner,
            _not_send_or_sync: _,
        } = self;
        owner
    }
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::{Mutex, OnceLock},
        task::{Context, Poll, Waker},
    };

    use super::*;
    use crate::{
        construction::{
            ConstructionContext, InputPosition, prepare_bound_optional, prepare_bound_required,
        },
        inject_wrapper::Inject,
        registration::provider::CleanupFuture,
    };

    static DROP_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    static SHUTDOWN_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    static SHUTDOWN_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    static CANCELLATION_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    static TRANSIENT_CHILD_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    static TRANSIENT_CHILD_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    static TRANSIENT_CHILD_CANCELLATION_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    static LEASE_RELEASE_ORDER: OnceLock<Mutex<Vec<&'static str>>> = OnceLock::new();
    static LEASE_RELEASE_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct First;
    struct Second;
    struct ShutdownFirst;
    struct ShutdownMiddle;
    struct ShutdownSecond;
    struct CancellationFirst;
    struct CancellationSecond;
    struct TransientChildOwner;
    struct TransientChildFirst;
    struct TransientChildSecond;
    struct TransientChildCancellationOwner;
    struct TransientChildCancellationLeaf;
    struct LeaseAncestor;
    struct LeaseChild;

    trait Mailer: Send + Sync {
        fn label(&self) -> &'static str;
        fn address(&self) -> usize;
    }

    struct SmtpMailer {
        label: &'static str,
    }

    impl Mailer for SmtpMailer {
        fn label(&self) -> &'static str {
            self.label
        }

        fn address(&self) -> usize {
            self as *const Self as usize
        }
    }

    fn project_smtp_mailer(value: &SmtpMailer) -> &(dyn Mailer + 'static) {
        value
    }

    fn record_shutdown(event: &'static str) {
        SHUTDOWN_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("shutdown order mutex should not be poisoned")
            .push(event);
    }

    fn record_cancellation(event: &'static str) {
        CANCELLATION_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("cancellation order mutex should not be poisoned")
            .push(event);
    }

    fn record_transient_child(event: &'static str) {
        TRANSIENT_CHILD_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("transient child order mutex should not be poisoned")
            .push(event);
    }

    fn record_transient_child_cancellation(event: &'static str) {
        TRANSIENT_CHILD_CANCELLATION_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("transient child cancellation mutex should not be poisoned")
            .push(event);
    }

    fn cleanup_shutdown_first() -> CleanupFuture {
        Box::pin(async { record_shutdown("cleanup:first") })
    }

    fn cleanup_shutdown_second() -> CleanupFuture {
        Box::pin(async { record_shutdown("cleanup:second") })
    }

    fn cleanup_transient_child_owner() -> CleanupFuture {
        Box::pin(async { record_transient_child("cleanup:owner") })
    }

    fn cleanup_transient_child_first() -> CleanupFuture {
        Box::pin(async { record_transient_child("cleanup:child:first") })
    }

    fn cleanup_transient_child_second() -> CleanupFuture {
        Box::pin(async { record_transient_child("cleanup:child:second") })
    }

    struct PendingCleanup;

    impl Future for PendingCleanup {
        type Output = ();

        fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
            record_cancellation("cleanup:started");
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }

    impl Drop for PendingCleanup {
        fn drop(&mut self) {
            record_cancellation("cleanup:future dropped");
        }
    }

    fn cleanup_pending_forever() -> CleanupFuture {
        Box::pin(PendingCleanup)
    }

    struct PendingTransientChildCleanup;

    impl Future for PendingTransientChildCleanup {
        type Output = ();

        fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
            record_transient_child_cancellation("cleanup:child:started");
            context.waker().wake_by_ref();
            Poll::Pending
        }
    }

    impl Drop for PendingTransientChildCleanup {
        fn drop(&mut self) {
            record_transient_child_cancellation("cleanup:child:future dropped");
        }
    }

    fn cleanup_transient_child_pending_forever() -> CleanupFuture {
        Box::pin(PendingTransientChildCleanup)
    }

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

    impl Drop for LeaseAncestor {
        fn drop(&mut self) {
            LEASE_RELEASE_ORDER
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("lease release order mutex should not be poisoned")
                .push("ancestor");
        }
    }

    impl Drop for LeaseChild {
        fn drop(&mut self) {
            LEASE_RELEASE_ORDER
                .get_or_init(|| Mutex::new(Vec::new()))
                .lock()
                .expect("lease release order mutex should not be poisoned")
                .push("child");
        }
    }

    impl Drop for ShutdownFirst {
        fn drop(&mut self) {
            record_shutdown("drop:first");
        }
    }

    impl Drop for ShutdownSecond {
        fn drop(&mut self) {
            record_shutdown("drop:second");
        }
    }

    impl Drop for ShutdownMiddle {
        fn drop(&mut self) {
            record_shutdown("drop:middle");
        }
    }

    impl Drop for CancellationFirst {
        fn drop(&mut self) {
            record_cancellation("drop:first");
        }
    }

    impl Drop for CancellationSecond {
        fn drop(&mut self) {
            record_cancellation("drop:second");
        }
    }

    impl Drop for TransientChildOwner {
        fn drop(&mut self) {
            record_transient_child("drop:owner");
        }
    }

    impl Drop for TransientChildFirst {
        fn drop(&mut self) {
            record_transient_child("drop:child:first");
        }
    }

    impl Drop for TransientChildSecond {
        fn drop(&mut self) {
            record_transient_child("drop:child:second");
        }
    }

    impl Drop for TransientChildCancellationOwner {
        fn drop(&mut self) {
            record_transient_child_cancellation("drop:owner");
        }
    }

    impl Drop for TransientChildCancellationLeaf {
        fn drop(&mut self) {
            record_transient_child_cancellation("drop:child");
        }
    }

    fn block_on<F>(future: F) -> F::Output
    where
        F: Future,
    {
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut future = std::pin::pin!(future);

        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
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
    fn ordinary_drop_releases_an_owner_before_its_transient_child_arena() {
        let _test = TRANSIENT_CHILD_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("transient child test mutex should not be poisoned");
        let events = TRANSIENT_CHILD_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        events
            .lock()
            .expect("transient child order mutex should not be poisoned")
            .clear();

        let mut child = Arena::new();
        child
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<TransientChildFirst>()),
                ErasedService::new(TransientChildFirst),
                Some(cleanup_transient_child_first),
            )
            .expect("transient child should commit");

        let mut owner = Arena::new();
        owner
            .commit_with_cleanup_and_children(
                ServiceIdentifier::from(ServiceType::create::<TransientChildOwner>()),
                ErasedService::new(TransientChildOwner),
                Some(cleanup_transient_child_owner),
                vec![child],
            )
            .expect("owner should accept its transient child arena");
        drop(owner);

        assert_eq!(
            *events
                .lock()
                .expect("transient child order mutex should not be poisoned"),
            ["drop:owner", "drop:child:first"],
            "ordinary Drop must not run hooks and must keep transient storage alive through the owner destructor"
        );
    }

    #[test]
    fn shutdown_runs_owner_hook_and_drop_before_transient_child_shutdowns() {
        let _test = TRANSIENT_CHILD_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("transient child test mutex should not be poisoned");
        let events = TRANSIENT_CHILD_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        events
            .lock()
            .expect("transient child order mutex should not be poisoned")
            .clear();

        let mut first_child = Arena::new();
        first_child
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<TransientChildFirst>()),
                ErasedService::new(TransientChildFirst),
                Some(cleanup_transient_child_first),
            )
            .expect("first transient child should commit");

        let mut second_child = Arena::new();
        second_child
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<TransientChildSecond>()),
                ErasedService::new(TransientChildSecond),
                Some(cleanup_transient_child_second),
            )
            .expect("second transient child should commit");

        let mut owner = Arena::new();
        owner
            .commit_with_cleanup_and_children(
                ServiceIdentifier::from(ServiceType::create::<TransientChildOwner>()),
                ErasedService::new(TransientChildOwner),
                Some(cleanup_transient_child_owner),
                vec![first_child, second_child],
            )
            .expect("owner should accept its transient child arenas");

        block_on(owner.shutdown());

        assert_eq!(
            *events
                .lock()
                .expect("transient child order mutex should not be poisoned"),
            [
                "cleanup:owner",
                "drop:owner",
                "cleanup:child:second",
                "drop:child:second",
                "cleanup:child:first",
                "drop:child:first",
            ],
            "explicit shutdown must finish each owner phase before moving into its child arenas"
        );
    }

    #[test]
    fn shutdown_runs_cleanup_then_drops_each_service_in_reverse_commit_order() {
        let _test = SHUTDOWN_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("shutdown test mutex should not be poisoned");
        let events = SHUTDOWN_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        events
            .lock()
            .expect("shutdown order mutex should not be poisoned")
            .clear();

        let mut arena = Arena::new();
        arena
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<ShutdownFirst>()),
                ErasedService::new(ShutdownFirst),
                Some(cleanup_shutdown_first),
            )
            .expect("first service should commit with its cleanup hook");
        arena
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<ShutdownMiddle>()),
                ErasedService::new(ShutdownMiddle),
                None,
            )
            .expect("middle service should commit without a cleanup hook");
        arena
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<ShutdownSecond>()),
                ErasedService::new(ShutdownSecond),
                Some(cleanup_shutdown_second),
            )
            .expect("second service should commit with its cleanup hook");

        block_on(arena.shutdown());

        assert_eq!(
            *events
                .lock()
                .expect("shutdown order mutex should not be poisoned"),
            [
                "cleanup:second",
                "drop:second",
                "drop:middle",
                "cleanup:first",
                "drop:first"
            ],
            "each hook must finish before its own value is destroyed, in reverse commit order"
        );
    }

    #[test]
    fn ordinary_drop_destroys_values_without_starting_cleanup_hooks() {
        let _test = SHUTDOWN_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("shutdown test mutex should not be poisoned");
        let events = SHUTDOWN_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        events
            .lock()
            .expect("shutdown order mutex should not be poisoned")
            .clear();

        {
            let mut arena = Arena::new();
            arena
                .commit_with_cleanup(
                    ServiceIdentifier::from(ServiceType::create::<ShutdownFirst>()),
                    ErasedService::new(ShutdownFirst),
                    Some(cleanup_shutdown_first),
                )
                .expect("service should commit with a cleanup hook");
        }

        assert_eq!(
            *events
                .lock()
                .expect("shutdown order mutex should not be poisoned"),
            ["drop:first"],
            "ordinary Arena Drop must not start asynchronous cleanup"
        );
    }

    #[test]
    fn cancelling_shutdown_does_not_finish_the_hook_but_drops_current_and_remaining_values() {
        let events = CANCELLATION_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        events
            .lock()
            .expect("cancellation order mutex should not be poisoned")
            .clear();

        let mut arena = Arena::new();
        arena
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<CancellationFirst>()),
                ErasedService::new(CancellationFirst),
                None,
            )
            .expect("first cancellation service should commit");
        arena
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<CancellationSecond>()),
                ErasedService::new(CancellationSecond),
                Some(cleanup_pending_forever),
            )
            .expect("second cancellation service should commit with its pending hook");

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut shutdown = Box::pin(arena.shutdown());
        assert!(matches!(
            shutdown.as_mut().poll(&mut context),
            Poll::Pending
        ));
        drop(shutdown);

        let events = events
            .lock()
            .expect("cancellation order mutex should not be poisoned")
            .clone();
        assert!(events.contains(&"cleanup:started"));
        assert!(events.contains(&"cleanup:future dropped"));
        assert!(events.contains(&"drop:second"));
        assert!(events.contains(&"drop:first"));
        assert!(
            events
                .iter()
                .position(|event| *event == "cleanup:future dropped")
                .expect("the pending cleanup future must be dropped")
                < events
                    .iter()
                    .position(|event| *event == "drop:second")
                    .expect("the current service must be destroyed"),
            "cancellation must drop the in-flight cleanup future before Arena Drop releases its value"
        );
        assert!(
            events
                .iter()
                .position(|event| *event == "drop:second")
                .expect("current service must be destroyed")
                < events
                    .iter()
                    .position(|event| *event == "drop:first")
                    .expect("remaining service must be destroyed"),
            "Arena Drop must preserve reverse destruction order after cancellation"
        );
    }

    #[test]
    fn cancelling_child_shutdown_still_drops_the_owner_and_transient_child() {
        let _test = TRANSIENT_CHILD_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("transient child test mutex should not be poisoned");
        let events = TRANSIENT_CHILD_CANCELLATION_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        events
            .lock()
            .expect("transient child cancellation mutex should not be poisoned")
            .clear();

        let mut child = Arena::new();
        child
            .commit_with_cleanup(
                ServiceIdentifier::from(ServiceType::create::<TransientChildCancellationLeaf>()),
                ErasedService::new(TransientChildCancellationLeaf),
                Some(cleanup_transient_child_pending_forever),
            )
            .expect("transient child should commit with a pending cleanup hook");

        let mut owner = Arena::new();
        owner
            .commit_with_cleanup_and_children(
                ServiceIdentifier::from(ServiceType::create::<TransientChildCancellationOwner>()),
                ErasedService::new(TransientChildCancellationOwner),
                None,
                vec![child],
            )
            .expect("owner should accept the transient child arena");

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut shutdown = Box::pin(owner.shutdown());
        assert!(matches!(
            shutdown.as_mut().poll(&mut context),
            Poll::Pending
        ));
        drop(shutdown);

        let events = events
            .lock()
            .expect("transient child cancellation mutex should not be poisoned")
            .clone();
        assert!(events.contains(&"drop:owner"));
        assert!(events.contains(&"cleanup:child:started"));
        assert!(events.contains(&"cleanup:child:future dropped"));
        assert!(events.contains(&"drop:child"));
        assert!(
            events
                .iter()
                .position(|event| *event == "drop:owner")
                .expect("owner must be dropped")
                < events
                    .iter()
                    .position(|event| *event == "cleanup:child:started")
                    .expect("child shutdown must begin after owner Drop"),
            "the owner value must be destroyed before its child shutdown begins"
        );
        assert!(
            events
                .iter()
                .position(|event| *event == "cleanup:child:future dropped")
                .expect("in-flight child cleanup future must be dropped")
                < events
                    .iter()
                    .position(|event| *event == "drop:child")
                    .expect("child value must be destroyed"),
            "cancelling shutdown must first drop the child cleanup future, then safely drop the child arena"
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

    #[test]
    fn worker_lease_set_keeps_a_prepared_inject_alive_after_the_facade_drops() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}

        assert_send::<ArenaLease>();
        assert_sync::<ArenaLease>();
        assert_send::<ArenaLeaseSet>();
        assert_sync::<ArenaLeaseSet>();

        let identifier = ServiceIdentifier::from(ServiceType::create::<String>());
        let leases = {
            let (leases, inject) = {
                let mut arena = Arena::new();
                arena
                    .insert(identifier, String::from("leased repository"))
                    .expect("service should commit before a worker lease is issued");

                let leases = ArenaLeaseSet::from_arenas([&arena]);
                let mut context = ConstructionContext::new();
                crate::construction::prepare_required::<String>(
                    &mut context,
                    InputPosition(0),
                    arena.lookup(identifier),
                )
                .expect("a committed service should prepare an input token");
                let inject = context
                    .take::<String>(InputPosition(0))
                    .expect("the prepared input should be consumable");
                (leases, inject)
            };

            assert_eq!(&*inject, "leased repository");
            leases
        };
        drop(leases);
    }

    #[test]
    fn worker_lease_set_releases_inner_backings_before_ancestor_backings() {
        let _guard = LEASE_RELEASE_TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        LEASE_RELEASE_ORDER
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .expect("lease release order mutex should not be poisoned")
            .clear();

        let leases = {
            let mut ancestor = Arena::new();
            ancestor
                .insert(
                    ServiceIdentifier::from(ServiceType::create::<LeaseAncestor>()),
                    LeaseAncestor,
                )
                .expect("ancestor service should commit before a lease is issued");

            let mut child = Arena::new();
            child
                .insert(
                    ServiceIdentifier::from(ServiceType::create::<LeaseChild>()),
                    LeaseChild,
                )
                .expect("child service should commit before a lease is issued");

            // This is the same outer-to-inner order used by the Tokio coordinator.  Both
            // facades disappear first, so only this lease set decides the backing drop order.
            ArenaLeaseSet::from_arenas([&ancestor, &child])
        };

        drop(leases);
        assert_eq!(
            *LEASE_RELEASE_ORDER
                .get()
                .expect("the two backing drops should record an order")
                .lock()
                .expect("lease release order mutex should not be poisoned"),
            ["child", "ancestor"],
            "a cancelled worker must release child/current Arena backing before an ancestor can shutdown"
        );
    }

    #[test]
    fn shutdown_waits_for_outstanding_worker_leases_before_releasing_values() {
        let drops = DROP_ORDER.get_or_init(|| Mutex::new(Vec::new()));
        drops
            .lock()
            .expect("drop order mutex should not be poisoned")
            .clear();

        let mut arena = Arena::new();
        arena
            .insert(
                ServiceIdentifier::from(ServiceType::create::<First>()),
                First,
            )
            .expect("service should commit before shutdown");
        let lease = arena.lease();

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        let mut shutdown = Box::pin(arena.shutdown());
        assert!(matches!(
            shutdown.as_mut().poll(&mut context),
            Poll::Pending
        ));
        assert!(
            drops
                .lock()
                .expect("drop order mutex should not be poisoned")
                .is_empty()
        );

        drop(lease);
        assert!(matches!(
            shutdown.as_mut().poll(&mut context),
            Poll::Ready(())
        ));
        assert_eq!(
            *drops
                .lock()
                .expect("drop order mutex should not be poisoned"),
            ["first"]
        );
    }

    #[test]
    fn bound_inputs_keep_the_trait_vtable_and_concrete_arena_address() {
        let identifier = ServiceIdentifier::from(ServiceType::create::<SmtpMailer>());
        let mut arena = Arena::new();
        arena
            .insert(identifier, SmtpMailer { label: "smtp" })
            .expect("concrete provider should commit to the arena");

        let mut required_context = ConstructionContext::new();
        prepare_bound_required::<SmtpMailer, dyn Mailer>(
            &mut required_context,
            InputPosition(0),
            arena.lookup(identifier),
            project_smtp_mailer,
        )
        .expect("typed projector should prepare the required trait input");
        let required: Inject<dyn Mailer> = required_context
            .take(InputPosition(0))
            .expect("required trait token should be consumable");

        let mut optional_context = ConstructionContext::new();
        prepare_bound_optional::<SmtpMailer, dyn Mailer>(
            &mut optional_context,
            InputPosition(0),
            arena.lookup(identifier),
            project_smtp_mailer,
        )
        .expect("typed projector should prepare the optional trait input");
        let optional: Inject<dyn Mailer> = optional_context
            .take_optional(InputPosition(0))
            .expect("optional trait token should be consumable")
            .expect("present concrete provider should yield a trait token");

        let concrete = arena
            .get::<SmtpMailer>()
            .expect("concrete provider should remain in the arena");
        assert_eq!(required.label(), "smtp");
        assert_eq!(optional.label(), "smtp");
        assert_eq!(required.address(), concrete.address());
        assert_eq!(optional.address(), concrete.address());
    }
}
