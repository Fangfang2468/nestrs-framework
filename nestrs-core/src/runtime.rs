//! `nestrs-core` 内部的单例激活运行时。
//!
//! 这个模块是 provider 元数据与 [`crate::arena::Arena`] 之间唯一的组合点。它先从
//! linkme 收集原始注册，再只为一个 concrete root 编译可达依赖图，最后按依赖后序
//! 把实例提交到 Arena。同步入口保持顺序激活；异步入口则把 independent 的就绪节点交给
//! 当前 Tokio runtime 的 worker 调度。它不提供动态解析：外部只能读取已经构建的 root
//! 或默认 key 的 concrete 持久实例，并可消费容器结束其生命周期。
#![allow(clippy::result_large_err)]
// `Compiler` below remains as a focused regression harness for the v0-v3 unit tests. The
// public runtime uses the occurrence-aware v4 compiler further down in this module.
#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    marker::PhantomData,
    rc::Rc,
};

use thiserror::Error;
use tokio::{runtime::Handle, task::JoinSet};

use crate::{
    arena::{Arena, ArenaError, ArenaLeaseSet, ArenaServiceRef},
    construction::{
        ActivationError, ConstructionContext, ErasedService, FactoryActivationFrame,
        FactoryConstructionContext, PrepareInput,
    },
    lifetime::Lifetime,
    registration::{
        binding::{REFLECTED_BINDINGS, TraitBinding},
        dependency::{Delivery, DependencyRequest, ProviderSource},
        injectable::Injectable,
        provider::{FactoryInvoker, Provider, ProviderCommon, REFLECTED_PROVIDERS},
        service_identifier::ServiceIdentifier,
        service_source::ServiceSource,
        service_type::ServiceType,
    },
};

/// 编译一个 root 可达 provider 图时发现的注册或能力错误。
///
/// 所有 identifier 都是精确 token，包含 service key；因此同一 concrete 类型的不同
/// key 不会在候选选择阶段彼此干扰。
#[derive(Debug, Error)]
pub(crate) enum CompileError {
    #[error("根服务 {root:?} 没有已注册的 provider")]
    MissingRoot { root: ServiceIdentifier },

    #[error("provider {provider:?} 的依赖 {dependency:?} 缺少可用 provider")]
    MissingDependency {
        provider: ServiceIdentifier,
        dependency: ServiceIdentifier,
        label: Option<&'static str>,
    },

    #[error("provider {provider:?} 的依赖 {dependency:?} 存在多个候选 provider：{candidates:?}")]
    AmbiguousDependency {
        provider: ServiceIdentifier,
        dependency: ServiceIdentifier,
        candidates: Vec<ServiceSource>,
    },

    #[error("provider {provider:?} 的 trait 依赖 {dependency:?} 缺少 #[bind] 声明")]
    MissingTraitBinding {
        provider: ServiceIdentifier,
        dependency: ServiceIdentifier,
    },

    #[error(
        "provider {provider:?} 的 trait 依赖 {dependency:?} 存在多个 #[bind] 声明：{candidates:?}"
    )]
    AmbiguousTraitBinding {
        provider: ServiceIdentifier,
        dependency: ServiceIdentifier,
        candidates: Vec<ServiceSource>,
    },

    #[error("provider 依赖出现循环：{chain:?}")]
    Cycle { chain: Vec<ServiceIdentifier> },

    #[error(
        "provider {provider:?}（{declaration:?}）使用了当前激活器不支持的生命周期 {lifetime:?}"
    )]
    UnsupportedLifetime {
        provider: ServiceIdentifier,
        lifetime: Lifetime,
        declaration: ServiceSource,
    },

    #[error(
        "root 服务 {root:?}（{declaration:?}）的生命周期为 {actual:?}，但该入口要求 {expected:?}"
    )]
    InvalidRootLifetime {
        root: ServiceIdentifier,
        expected: Lifetime,
        actual: Lifetime,
        declaration: ServiceSource,
    },

    #[error(
        "{provider_lifetime:?} provider {provider:?}（{provider_declaration:?}）不能依赖 {dependency_lifetime:?} provider {dependency:?}（{dependency_declaration:?}）"
    )]
    LifetimeInversion {
        provider: ServiceIdentifier,
        provider_lifetime: Lifetime,
        provider_declaration: ServiceSource,
        dependency: ServiceIdentifier,
        dependency_lifetime: Lifetime,
        dependency_declaration: ServiceSource,
    },

    #[error(
        "{owner_lifetime:?} owner {owner:?}（{owner_declaration:?}）持有的 transient 子树不能依赖 Scoped provider {dependency:?}（{dependency_declaration:?}）：{chain:?}"
    )]
    TransientLifetimeInversion {
        owner: ServiceIdentifier,
        owner_lifetime: Lifetime,
        owner_declaration: ServiceSource,
        dependency: ServiceIdentifier,
        dependency_declaration: ServiceSource,
        chain: Vec<ServiceIdentifier>,
    },

    #[error(
        "scope root {root:?}（{declaration:?}）已经由祖先 scope root {ancestor_root:?}（{ancestor_declaration:?}）拥有，不能作为新的 child scope root"
    )]
    ScopeRootOwnedByAncestor {
        root: ServiceIdentifier,
        declaration: ServiceSource,
        ancestor_root: ServiceIdentifier,
        ancestor_declaration: ServiceSource,
    },

    #[error(
        "scope root {provider_scope_root:?} 中的 provider {provider:?}（{provider_declaration:?}）不能依赖后代 scope root {dependency_scope_root:?} 中的 Scoped provider {dependency:?}（{dependency_declaration:?}）"
    )]
    ScopeLifetimeInversion {
        provider: ServiceIdentifier,
        provider_declaration: ServiceSource,
        provider_scope_root: ServiceIdentifier,
        dependency: ServiceIdentifier,
        dependency_declaration: ServiceSource,
        dependency_scope_root: ServiceIdentifier,
    },

    #[error(
        "factory provider {factory:?}（{factory_declaration:?}）的 transient 参数子树包含 cleanup provider {provider:?}（{declaration:?}）：{chain:?}"
    )]
    TransientFactoryParameterCleanup {
        factory: ServiceIdentifier,
        factory_declaration: ServiceSource,
        provider: ServiceIdentifier,
        declaration: ServiceSource,
        chain: Vec<ServiceIdentifier>,
    },

    #[error("provider {provider:?}（{declaration:?}）使用了 async factory，不能由同步激活入口激活")]
    UnsupportedAsyncFactory {
        provider: ServiceIdentifier,
        declaration: ServiceSource,
    },

    #[error("provider {provider:?}（{declaration:?}）声明了当前激活器不支持的 cleanup hook")]
    UnsupportedCleanup {
        provider: ServiceIdentifier,
        declaration: ServiceSource,
    },

    #[error(
        "闭合泛型 {requested:?} 的 materialize callback 返回了错误 token {provided:?}（{declaration:?}）"
    )]
    InvalidMaterializedProvider {
        requested: ServiceIdentifier,
        provided: ServiceIdentifier,
        declaration: ServiceSource,
    },

    #[error("provider {provider:?} 的构造输入布局无效：{reason}")]
    InvalidInputLayout {
        provider: ServiceIdentifier,
        reason: &'static str,
    },
}

/// 容器构建或 scope 激活失败。
///
/// 这是应用开发者唯一需要处理的运行时错误。其 `Display` 输出保留 provider 声明与
/// 失败原因，但 Compiler、Arena 和宏构造 ABI 的实现错误不会成为可匹配的公开类型。
pub struct BuildError {
    detail: BuildErrorDetail,
}

/// 仅供 runtime 在构建 [`BuildError`] 时保留诊断上下文。
///
/// factory 的 `Err(E)` 仍由既有 ABI 归一化为 [`ActivationError::FactoryFailed`]，不会
/// 泄漏用户错误值或改变 factory 边界。
#[derive(Debug, Error)]
enum BuildErrorDetail {
    #[error(transparent)]
    Compile(#[from] CompileError),

    #[error("激活 provider {provider:?}（{declaration:?}）时失败")]
    Activation {
        provider: ServiceIdentifier,
        declaration: ServiceSource,
        #[source]
        error: ActivationError,
    },

    #[error("向 Arena 提交 provider {provider:?}（{declaration:?}）时失败")]
    Arena {
        provider: ServiceIdentifier,
        declaration: ServiceSource,
        #[source]
        error: ArenaError,
    },

    #[error("异步激活必须在活动 Tokio runtime 中执行")]
    TokioRuntimeUnavailable,

    #[error("激活 worker 在 provider {provider:?}（{declaration:?}）执行期间 panic")]
    ActivationWorkerPanicked {
        provider: ServiceIdentifier,
        declaration: ServiceSource,
    },

    #[error("激活 worker 在 provider {provider:?}（{declaration:?}）执行期间被取消")]
    ActivationWorkerCancelled {
        provider: ServiceIdentifier,
        declaration: ServiceSource,
    },
}

impl BuildError {
    fn activation(
        provider: ServiceIdentifier,
        declaration: ServiceSource,
        error: ActivationError,
    ) -> Self {
        Self {
            detail: BuildErrorDetail::Activation {
                provider,
                declaration,
                error,
            },
        }
    }

    fn arena(provider: ServiceIdentifier, declaration: ServiceSource, error: ArenaError) -> Self {
        Self {
            detail: BuildErrorDetail::Arena {
                provider,
                declaration,
                error,
            },
        }
    }

    fn tokio_runtime_unavailable() -> Self {
        Self {
            detail: BuildErrorDetail::TokioRuntimeUnavailable,
        }
    }

    fn activation_worker_panicked(provider: ServiceIdentifier, declaration: ServiceSource) -> Self {
        Self {
            detail: BuildErrorDetail::ActivationWorkerPanicked {
                provider,
                declaration,
            },
        }
    }

    fn activation_worker_cancelled(
        provider: ServiceIdentifier,
        declaration: ServiceSource,
    ) -> Self {
        Self {
            detail: BuildErrorDetail::ActivationWorkerCancelled {
                provider,
                declaration,
            },
        }
    }
}

impl From<CompileError> for BuildError {
    fn from(error: CompileError) -> Self {
        Self {
            detail: BuildErrorDetail::Compile(error),
        }
    }
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.detail.fmt(formatter)
    }
}

impl std::fmt::Debug for BuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BuildError")
            .field("message", &self.detail.to_string())
            .finish()
    }
}

impl std::error::Error for BuildError {}

/// 静态 scope 链的结束标记。
///
/// 它只出现在 [`ScopeLayer`] 的类型参数中，不表示可在运行时创建或解析的服务。
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopeEnd;

/// 静态声明的一层 Scoped root 及其可选 child scope 链。
///
/// 例如 `ScopeLayer<RequestRoot, ScopeLayer<TransactionRoot>>` 表示 request 后可创建
/// transaction child scope。链在 provider 构建时一次性编译；它不是运行时 service
/// locator 或动态注册 API。
#[derive(Debug, Clone, Copy, Default)]
pub struct ScopeLayer<Root, Child = ScopeEnd>(PhantomData<fn() -> (Root, Child)>);

mod scope_chain_sealed {
    pub trait Sealed {}
}

/// `ScopeLayer` 链尾的内部类型约束。
///
/// 该 trait 被密封，调用方只需组合 [`ScopeLayer`] 与 [`ScopeEnd`]，无需实现它。
#[doc(hidden)]
pub trait ScopeTail: scope_chain_sealed::Sealed {
    fn collect_scope_roots(roots: &mut Vec<ServiceIdentifier>);
}

/// 可作为 [`crate::scope::ScopeProvider`] 参数的静态 Scope 链。
///
/// 该 trait 被密封；公开关联类型只用于让返回的 provider 和第一个
/// [`Scope`] 保持类型化。
#[doc(hidden)]
pub trait ScopeChain: ScopeTail {
    type Root: Send + Sync + 'static;
    type Child: ScopeTail;
}

impl scope_chain_sealed::Sealed for ScopeEnd {}

impl ScopeTail for ScopeEnd {
    fn collect_scope_roots(_roots: &mut Vec<ServiceIdentifier>) {}
}

impl<Root, Child> scope_chain_sealed::Sealed for ScopeLayer<Root, Child>
where
    Root: Send + Sync + 'static,
    Child: ScopeTail,
{
}

impl<Root, Child> ScopeTail for ScopeLayer<Root, Child>
where
    Root: Send + Sync + 'static,
    Child: ScopeTail,
{
    fn collect_scope_roots(roots: &mut Vec<ServiceIdentifier>) {
        roots.push(ServiceIdentifier::from(ServiceType::create::<Root>()));
        Child::collect_scope_roots(roots);
    }
}

impl<Root, Child> ScopeChain for ScopeLayer<Root, Child>
where
    Root: Send + Sync + 'static,
    Child: ScopeTail,
{
    type Root = Root;
    type Child = Child;
}

/// 一个已构建 root 的单例服务容器。
///
/// 它公开构建目标 root、读取已提交 concrete 服务与消费式 shutdown。所有依赖仍由私有
/// `Arena` 持有；[`Self::get`] 不会按需解析或激活服务。
pub struct ServiceProvider<Root> {
    arena: Arena,
    _root: PhantomData<fn() -> Root>,
    // The Tokio scheduler may use worker threads internally, but the container facade is not a
    // cross-thread service-locator or ownership API.
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl<Root> ServiceProvider<Root>
where
    Root: Send + Sync + 'static,
{
    /// 收集当前链接单元的 provider 注册，并 eager 构建默认 key 的 `Root`。
    ///
    /// 只有 `Root` 可达的 provider 会被校验或激活；无关的错误注册不会阻止该 root
    /// 闭环启动。每次调用都会创建独立的 Arena 和独立的 singleton 实例集。
    ///
    /// `Root` 本身必须存在于 linkme 收集到的显式 provider 中。闭合泛型的
    /// `ProviderDefinition` fallback 只适用于已选 provider 的依赖请求，不能从 root
    /// 类型反推一个泛型 family。
    pub fn build() -> Result<Self, BuildError> {
        let registry = ProviderRegistry::collect();
        let graph = ActivationCompiler::new(registry).compile_root::<Root>()?;
        let arena = activate_v4(&graph)?;

        Ok(Self {
            arena,
            _root: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    /// 异步收集、编译并 eager 激活默认 key 的 `Root`。
    ///
    /// 与 [`Self::build`] 不同，这个入口允许 root 可达闭包中的 async factory。它必须在
    /// 活动 Tokio runtime 内调用，否则返回带相应诊断的 [`BuildError`]；
    /// current-thread runtime 合法但只提供并发推进，multi-thread runtime 可以让互不依赖
    /// 的就绪节点在 worker 上并行执行。依赖已提交后，所有新就绪的 provider 都交由
    /// Tokio `JoinSet` 调度，包含同步 constructor 与同步 factory。
    ///
    /// 返回的 future 与完成后的 provider 都不承诺 [`Send`] 或 [`Sync`]。取消这个 future
    /// 会停止新调度并 abort 在飞 worker；Tokio abort 不等待，因此 lease 会保活相关
    /// Arena backing，直至 worker 的 frame、future 与 output 实际析构。这个路径只做
    /// Rust 析构，不会运行 cleanup hook。
    pub async fn build_async() -> Result<Self, BuildError> {
        let registry = ProviderRegistry::collect();
        let graph = ActivationCompiler::new(registry).compile_async_root::<Root>()?;
        let arena = activate_v4_async(&graph).await?;

        Ok(Self {
            arena,
            _root: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    /// 消费此 provider，并按反向提交顺序运行其已声明的 cleanup hook。
    ///
    /// cleanup 只在这个显式异步入口中得到执行保证；直接丢弃 provider、构建失败或
    /// 取消构建 future 时，Arena 只进行 Rust 析构而不会隐式驱动异步 hook。若这个
    /// shutdown future 本身被取消，当前与尚未开始的 hook 也不保证完成，但 Arena 仍会
    /// 安全析构全部已提交服务。
    pub async fn shutdown(self) {
        self.arena.shutdown().await;
    }

    /// 返回构建时指定的 root 服务。
    ///
    /// `build` 在成功返回前已经提交这个精确 token；这里使用 `expect` 是内部不变量，
    /// 而不是额外的解析路径。
    pub fn root(&self) -> &Root {
        self.arena
            .get::<Root>()
            .expect("a successful ServiceProvider build must commit its root")
    }

    /// 读取已经提交到这个 root 闭包的默认 key concrete 服务。
    ///
    /// 这只是类型化只读查询：它不会按需解析、创建或缓存服务。trait、keyed 服务和
    /// Transient 没有可唯一返回的 persistent token，都会得到 `None`。
    pub fn get<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.arena.get::<T>().ok()
    }
}

/// 一个应用 Singleton root 与预编译静态 Scope 链的高级容器。
///
/// `Chain` 决定可创建的第一层 root 和所有后代层。它只能读取已经提交的默认 key
/// concrete Singleton，不提供动态 token 解析。
pub struct ScopeProvider<AppRoot, Chain>
where
    AppRoot: Send + Sync + 'static,
    Chain: ScopeChain,
{
    arena: Arena,
    plan: ActivationScopePlan,
    _roots: PhantomData<fn() -> (AppRoot, Chain)>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl<AppRoot, Chain> ScopeProvider<AppRoot, Chain>
where
    AppRoot: Send + Sync + 'static,
    Chain: ScopeChain,
{
    /// 收集应用 root 与静态 Scope 链的可达闭包，并 eager 激活共享 Singleton 子图。
    ///
    /// 每个 scope root 都由 [`ScopeLayer`] 固定，child scope 创建时不会重新扫描注册或
    /// 动态补建父容器。
    pub fn build() -> Result<Self, BuildError> {
        let registry = ProviderRegistry::collect();
        let plan = ActivationCompiler::new(registry).compile_scope_chain::<AppRoot, Chain>()?;
        let arena = activate_v4_scope_singletons(&plan)?;

        Ok(Self {
            arena,
            plan,
            _roots: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    /// [`Self::build`] 的 Tokio 异步版本。
    ///
    /// 它允许整条静态链可达的 async factory 与 cleanup，并要求活动 Tokio runtime；
    /// current-thread runtime 并发推进，multi-thread runtime 可并行运行独立节点。
    pub async fn build_async() -> Result<Self, BuildError> {
        let registry = ProviderRegistry::collect();
        let plan =
            ActivationCompiler::new(registry).compile_async_scope_chain::<AppRoot, Chain>()?;
        let arena = activate_v4_scope_singletons_async(&plan).await?;

        Ok(Self {
            arena,
            plan,
            _roots: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    /// 返回启动时提交的 Singleton 应用 root。
    pub fn root(&self) -> &AppRoot {
        self.arena
            .get::<AppRoot>()
            .expect("a successful ScopeProvider build must commit its app root")
    }

    /// 读取当前应用闭包中已提交的默认 key concrete Singleton。
    ///
    /// 这个查询不会创建服务；Scoped、Transient、trait 与 keyed 服务均不会通过该入口
    /// 返回。
    pub fn get<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.arena.get::<T>().ok()
    }

    /// eager 构建链头声明的第一层 Scoped root。
    pub fn create_scope(&self) -> Result<Scope<'_, Chain::Root, Chain::Child>, BuildError> {
        Scope::create_root(&self.plan, &self.arena)
    }

    /// 异步 eager 构建链头声明的第一层 Scoped root。
    pub async fn create_scope_async(
        &self,
    ) -> Result<Scope<'_, Chain::Root, Chain::Child>, BuildError> {
        Scope::create_root_async(&self.plan, &self.arena).await
    }

    /// 消费此 provider，并按反向提交顺序 shutdown Singleton Arena。
    ///
    /// 所有派生的 [`Scope`] 都经借用链绑定到本值，故它们必须先结束；每一层 Scope 的
    /// cleanup 仍须由该 scope 自己的 [`Scope::shutdown`] 驱动。
    pub async fn shutdown(self) {
        self.arena.shutdown().await;
    }
}

/// 一个借用其父 [`ScopeProvider`] 的 Scoped 服务容器。
///
/// 这个借用确保 scope 内保存的 `Inject<Singleton>` token 不会超过父 Singleton Arena。
/// scope 析构时，自己的 Arena 会按提交逆序释放所有 Scoped 服务。
pub struct Scope<'provider, Root, Tail = ScopeEnd>
where
    Root: Send + Sync + 'static,
    Tail: ScopeTail,
{
    arena: Arena,
    singleton_arena: &'provider Arena,
    plan: &'provider ActivationScopePlan,
    /// Arena 的下标与 `ScopeFrameId` 相同，只包含祖先层，绝不包含自己的 `arena`。
    /// 这避免 self-reference，同时让 child 可以精确读取任何祖先 Scoped 输入。
    ancestor_arenas: Vec<&'provider Arena>,
    frame: ScopeFrameId,
    _root: PhantomData<fn() -> Root>,
    _tail: PhantomData<fn() -> Tail>,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'provider, Root, Tail> Scope<'provider, Root, Tail>
where
    Root: Send + Sync + 'static,
    Tail: ScopeTail,
{
    fn create_root(
        plan: &'provider ActivationScopePlan,
        singleton_arena: &'provider Arena,
    ) -> Result<Self, BuildError> {
        let frame = ScopeFrameId(0);
        let arena = activate_v4_scope_frame(plan, singleton_arena, &[], frame)?;
        Ok(Self {
            arena,
            singleton_arena,
            plan,
            ancestor_arenas: Vec::new(),
            frame,
            _root: PhantomData,
            _tail: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    async fn create_root_async(
        plan: &'provider ActivationScopePlan,
        singleton_arena: &'provider Arena,
    ) -> Result<Self, BuildError> {
        let frame = ScopeFrameId(0);
        let arena = activate_v4_scope_frame_async(plan, singleton_arena, &[], frame).await?;
        Ok(Self {
            arena,
            singleton_arena,
            plan,
            ancestor_arenas: Vec::new(),
            frame,
            _root: PhantomData,
            _tail: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    /// 返回这个 scope eager 构建的 Scoped root。
    pub fn root(&self) -> &Root {
        self.arena
            .get::<Root>()
            .expect("a successful Scope build must commit its root")
    }

    /// 读取当前 scope、精确祖先 scope 或 Singleton Arena 中已提交的默认 key concrete
    /// 服务。查询绝不触发动态解析；Transient、trait 与 keyed 服务始终不可由此读取。
    pub fn get<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.arena
            .get::<T>()
            .ok()
            .or_else(|| {
                self.ancestor_arenas
                    .iter()
                    .rev()
                    .find_map(|arena| arena.get::<T>().ok())
            })
            .or_else(|| self.singleton_arena.get::<T>().ok())
    }

    /// 消费当前 scope，并按反向提交顺序显式运行它的 Scoped cleanup hook。
    ///
    /// 父 [`ScopeProvider`] 的 Singleton Arena 不受影响；其 hook 只会在父 provider
    /// 自己被 [`ScopeProvider::shutdown`] 消费时执行。
    pub async fn shutdown(self) {
        self.arena.shutdown().await;
    }

    fn child_ancestor_arenas<'child>(&'child self) -> Vec<&'child Arena> {
        let mut ancestors: Vec<&'child Arena> = self.ancestor_arenas.to_vec();
        ancestors.push(&self.arena);
        ancestors
    }
}

impl<'provider, Root, ChildRoot, Grandchild>
    Scope<'provider, Root, ScopeLayer<ChildRoot, Grandchild>>
where
    Root: Send + Sync + 'static,
    ChildRoot: Send + Sync + 'static,
    Grandchild: ScopeTail,
{
    /// eager 构建静态链中紧随当前层的 child scope。
    ///
    /// child 只借用当前 scope 与其祖先，因此它必须先结束，当前 scope 才能 shutdown 或
    /// 被销毁。该 API 没有 `T` 参数，child root 始终由 [`ScopeLayer`] 固定。
    pub fn create_child_scope<'child>(
        &'child self,
    ) -> Result<Scope<'child, ChildRoot, Grandchild>, BuildError> {
        let frame = ScopeFrameId(
            self.frame
                .0
                .checked_add(1)
                .expect("scope chain depth must fit in usize"),
        );
        let ancestor_arenas = self.child_ancestor_arenas();
        let arena =
            activate_v4_scope_frame(self.plan, self.singleton_arena, &ancestor_arenas, frame)?;
        Ok(Scope {
            arena,
            singleton_arena: self.singleton_arena,
            plan: self.plan,
            ancestor_arenas,
            frame,
            _root: PhantomData,
            _tail: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }

    /// [`Self::create_child_scope`] 的 Tokio 异步版本。
    ///
    /// 它要求活动 Tokio runtime；current-thread runtime 安全地并发推进，multi-thread
    /// runtime 可并行运行互不依赖的当前层 provider。child scope 与公开容器仍不承诺
    /// `Send` 或 `Sync`。
    pub async fn create_child_scope_async<'child>(
        &'child self,
    ) -> Result<Scope<'child, ChildRoot, Grandchild>, BuildError> {
        let frame = ScopeFrameId(
            self.frame
                .0
                .checked_add(1)
                .expect("scope chain depth must fit in usize"),
        );
        let ancestor_arenas = self.child_ancestor_arenas();
        let arena =
            activate_v4_scope_frame_async(self.plan, self.singleton_arena, &ancestor_arenas, frame)
                .await?;
        Ok(Scope {
            arena,
            singleton_arena: self.singleton_arena,
            plan: self.plan,
            ancestor_arenas,
            frame,
            _root: PhantomData,
            _tail: PhantomData,
            _not_send_or_sync: PhantomData,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ProviderId(usize);

/// linkme 注册的稳定索引。
///
/// 保留原始 `Provider::{Class, Factory}` payload，不把它们归一化成另一套 activation
/// 模型。只有 compiler 成功选择某个 provider 后，才将它写入不可变 compiled graph。
struct ProviderRegistry {
    providers: Vec<Provider>,
    explicit_by_identifier: BTreeMap<ServiceIdentifier, Vec<ProviderId>>,
    materialized_by_identifier: BTreeMap<ServiceIdentifier, ProviderId>,
    bindings_by_trait: BTreeMap<ServiceType, Vec<TraitBinding>>,
}

impl ProviderRegistry {
    fn collect() -> Self {
        Self::from_parts(
            REFLECTED_PROVIDERS
                .iter()
                .map(|provider| provider())
                .collect(),
            REFLECTED_BINDINGS.iter().map(|binding| binding()).collect(),
        )
    }

    fn from_parts(mut providers: Vec<Provider>, mut bindings: Vec<TraitBinding>) -> Self {
        providers.sort_by(|left, right| {
            provider_common(left)
                .source
                .cmp(&provider_common(right).source)
                .then_with(|| provider_identifier(left).cmp(&provider_identifier(right)))
        });
        bindings.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.trait_type.cmp(&right.trait_type))
                .then_with(|| left.concrete_type.cmp(&right.concrete_type))
        });

        let mut explicit_by_identifier: BTreeMap<ServiceIdentifier, Vec<ProviderId>> =
            BTreeMap::new();
        for (index, provider) in providers.iter().enumerate() {
            explicit_by_identifier
                .entry(provider_identifier(provider))
                .or_default()
                .push(ProviderId(index));
        }

        let mut bindings_by_trait: BTreeMap<ServiceType, Vec<TraitBinding>> = BTreeMap::new();
        for binding in bindings {
            bindings_by_trait
                .entry(binding.trait_type)
                .or_default()
                .push(binding);
        }

        Self {
            providers,
            explicit_by_identifier,
            materialized_by_identifier: BTreeMap::new(),
            bindings_by_trait,
        }
    }

    fn provider(&self, id: ProviderId) -> &Provider {
        &self.providers[id.0]
    }

    fn select_explicit(
        &self,
        identifier: ServiceIdentifier,
    ) -> Result<Option<ProviderId>, Vec<ServiceSource>> {
        let Some(candidates) = self.explicit_by_identifier.get(&identifier) else {
            return Ok(None);
        };

        if candidates.len() == 1 {
            return Ok(Some(candidates[0]));
        }

        let primary: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|candidate| provider_common(self.provider(*candidate)).primary)
            .collect();
        if primary.len() == 1 {
            return Ok(Some(primary[0]));
        }

        Err(candidates
            .iter()
            .map(|candidate| provider_common(self.provider(*candidate)).source)
            .collect())
    }

    fn select_binding(
        &self,
        trait_type: ServiceType,
    ) -> Result<Option<TraitBinding>, Vec<ServiceSource>> {
        let Some(bindings) = self.bindings_by_trait.get(&trait_type) else {
            return Ok(None);
        };

        if bindings.len() == 1 {
            return Ok(Some(bindings[0]));
        }

        Err(bindings.iter().map(|binding| binding.source).collect())
    }

    fn materialize(
        &mut self,
        requested: ServiceIdentifier,
        callback: crate::registration::dependency::ClosedProviderCallback,
    ) -> Result<ProviderId, CompileError> {
        if let Some(provider) = self.materialized_by_identifier.get(&requested) {
            return Ok(*provider);
        }

        let provider = callback();
        let provided = provider_identifier(&provider);
        let declaration = provider_common(&provider).source;
        if provided != requested {
            return Err(CompileError::InvalidMaterializedProvider {
                requested,
                provided,
                declaration,
            });
        }

        let id = ProviderId(self.providers.len());
        self.providers.push(provider);
        self.materialized_by_identifier.insert(requested, id);
        Ok(id)
    }
}

/// 编译后的一项输入边。
///
/// `dependency == None` 是已证明可缺席的 optional 输入；激活时仍调用 consumer 已
/// 单态化的 `PrepareInput`，这样 `Option<Inject<T>>` 的类型证明不会被 runtime 擦除。
#[derive(Clone, Copy)]
struct CompiledInput {
    position: crate::construction::InputPosition,
    dependency: Option<ServiceIdentifier>,
    storage: InputStorage,
    prepare: PrepareInput,
}

/// 一个已解析输入从哪个 Arena 读取服务。
///
/// scope 激活不做“先本地、再父级”的隐式回退：编译阶段已经依据所选 concrete
/// provider 的生命周期写入来源，因此 key、trait projector 与 optional 语义不会在
/// runtime 被重新解释。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputStorage {
    Absent,
    Singleton,
    Scoped,
}

struct CompiledProvider {
    provider: Provider,
    inputs: Vec<CompiledInput>,
}

/// 不可变的、仅包含 root 可达节点的 provider 图。
///
/// 图的公共实现细节不会暴露；`order` 是依赖后序，因此 runtime 可以不进行二次解析
/// 地直接顺序激活。
struct CompiledProviderGraph {
    root: ServiceIdentifier,
    order: Vec<ServiceIdentifier>,
    providers: BTreeMap<ServiceIdentifier, CompiledProvider>,
}

/// 一个应用 root 与一个 scope root 共享的不可变编译计划。
///
/// `singleton_order` 在 provider 构建时执行一次；`scoped_order` 在每次创建 scope 时
/// 执行一次。两者引用同一份 provider 选择结果，以便两根共享的闭合泛型只物化一次。
struct CompiledScopePlan {
    app_root: ServiceIdentifier,
    scope_root: ServiceIdentifier,
    singleton_order: Vec<ServiceIdentifier>,
    scoped_order: Vec<ServiceIdentifier>,
    providers: BTreeMap<ServiceIdentifier, CompiledProvider>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VisitState {
    Visiting,
    Done,
}

/// 编译出的图允许使用哪些激活能力。
///
/// 同一个选择/依赖图在同步与异步入口之间保持一致；差异仅在 async factory 与 cleanup
/// hook 是否可达。这样 `build()` 不会因为新增异步 shutdown 支持而静默改变既有的失败
/// 契约。
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivationCapability {
    SyncOnly,
    AsyncFactoriesAndCleanup,
}

/// 当前入口允许哪些 provider 生命周期进入可达图。
#[derive(Clone, Copy, PartialEq, Eq)]
enum LifetimeCapability {
    SingletonOnly,
    SingletonAndScoped,
}

/// 从精确 root token 编译选择、绑定和激活顺序的私有编译器。
struct Compiler {
    registry: ProviderRegistry,
    activation_capability: ActivationCapability,
    lifetime_capability: LifetimeCapability,
    states: BTreeMap<ServiceIdentifier, VisitState>,
    stack: Vec<ServiceIdentifier>,
    order: Vec<ServiceIdentifier>,
    providers: BTreeMap<ServiceIdentifier, CompiledProvider>,
}

impl Compiler {
    fn new(registry: ProviderRegistry) -> Self {
        Self {
            registry,
            activation_capability: ActivationCapability::SyncOnly,
            lifetime_capability: LifetimeCapability::SingletonOnly,
            states: BTreeMap::new(),
            stack: Vec::new(),
            order: Vec::new(),
            providers: BTreeMap::new(),
        }
    }

    fn compile_root<Root>(mut self) -> Result<CompiledProviderGraph, CompileError>
    where
        Root: Injectable,
    {
        let (root, provider) = self.select_root::<Root>()?;

        self.compile_selected(root, provider)?;
        Ok(CompiledProviderGraph {
            root,
            order: self.order,
            providers: self.providers,
        })
    }

    fn compile_async_root<Root>(mut self) -> Result<CompiledProviderGraph, CompileError>
    where
        Root: Injectable,
    {
        self.activation_capability = ActivationCapability::AsyncFactoriesAndCleanup;
        self.compile_root::<Root>()
    }

    /// 编译一个 Singleton app root 与一个 Scoped request root 的可达并集。
    ///
    /// 两个 root 共用同一个 compiler/registry 会话，因此显式候选选择、trait binding
    /// 与闭合泛型 fallback 在整张双根图中保持一致。
    fn compile_scope_plan<AppRoot, ScopeRoot>(mut self) -> Result<CompiledScopePlan, CompileError>
    where
        AppRoot: Injectable,
        ScopeRoot: Injectable,
    {
        self.lifetime_capability = LifetimeCapability::SingletonAndScoped;
        self.compile_scope_roots::<AppRoot, ScopeRoot>()
    }

    fn compile_async_scope_plan<AppRoot, ScopeRoot>(
        mut self,
    ) -> Result<CompiledScopePlan, CompileError>
    where
        AppRoot: Injectable,
        ScopeRoot: Injectable,
    {
        self.activation_capability = ActivationCapability::AsyncFactoriesAndCleanup;
        self.lifetime_capability = LifetimeCapability::SingletonAndScoped;
        self.compile_scope_roots::<AppRoot, ScopeRoot>()
    }

    fn compile_scope_roots<AppRoot, ScopeRoot>(mut self) -> Result<CompiledScopePlan, CompileError>
    where
        AppRoot: Injectable,
        ScopeRoot: Injectable,
    {
        let (app_root, app_provider) = self.select_root::<AppRoot>()?;
        self.ensure_root_lifetime(app_root, app_provider, Lifetime::Singleton)?;
        self.compile_selected(app_root, app_provider)?;

        let (scope_root, scope_provider) = self.select_root::<ScopeRoot>()?;
        self.ensure_root_lifetime(scope_root, scope_provider, Lifetime::Scoped)?;
        self.compile_selected(scope_root, scope_provider)?;

        self.annotate_scope_inputs()?;

        let singleton_order = self
            .order
            .iter()
            .copied()
            .filter(|identifier| {
                provider_common(
                    &self
                        .providers
                        .get(identifier)
                        .expect("compiled provider order must reference a provider")
                        .provider,
                )
                .lifetime
                    == Lifetime::Singleton
            })
            .collect();
        let scoped_order = self
            .order
            .iter()
            .copied()
            .filter(|identifier| {
                provider_common(
                    &self
                        .providers
                        .get(identifier)
                        .expect("compiled provider order must reference a provider")
                        .provider,
                )
                .lifetime
                    == Lifetime::Scoped
            })
            .collect();

        Ok(CompiledScopePlan {
            app_root,
            scope_root,
            singleton_order,
            scoped_order,
            providers: self.providers,
        })
    }

    fn select_root<Root>(&self) -> Result<(ServiceIdentifier, ProviderId), CompileError>
    where
        Root: Injectable,
    {
        let root = ServiceIdentifier::from(ServiceType::create::<Root>());
        let Some(provider) = self.registry.select_explicit(root).map_err(|candidates| {
            CompileError::AmbiguousDependency {
                provider: root,
                dependency: root,
                candidates,
            }
        })?
        else {
            return Err(CompileError::MissingRoot { root });
        };

        Ok((root, provider))
    }

    fn ensure_root_lifetime(
        &self,
        root: ServiceIdentifier,
        provider: ProviderId,
        expected: Lifetime,
    ) -> Result<(), CompileError> {
        let common = provider_common(self.registry.provider(provider));
        if common.lifetime != expected {
            return Err(CompileError::InvalidRootLifetime {
                root,
                expected,
                actual: common.lifetime,
                declaration: common.source,
            });
        }
        Ok(())
    }

    /// 双根图完成后，为每条已解析输入标注其 Arena 来源，并验证生命周期不能向更短
    /// 生命周期捕获。这个阶段在 trait/key/generic 选择之后执行，所以检查的是最终的
    /// concrete edge，而不是原始语法请求。
    fn annotate_scope_inputs(&mut self) -> Result<(), CompileError> {
        let provider_lifetimes: BTreeMap<_, _> = self
            .providers
            .iter()
            .map(|(identifier, compiled)| {
                let common = provider_common(&compiled.provider);
                (*identifier, (common.lifetime, common.source))
            })
            .collect();

        for (identifier, compiled) in &mut self.providers {
            let common = provider_common(&compiled.provider);
            for input in &mut compiled.inputs {
                let Some(dependency) = input.dependency else {
                    input.storage = InputStorage::Absent;
                    continue;
                };
                let (dependency_lifetime, dependency_declaration) = provider_lifetimes
                    .get(&dependency)
                    .copied()
                    .expect("compiled input must reference a compiled provider");

                if common.lifetime == Lifetime::Singleton && dependency_lifetime == Lifetime::Scoped
                {
                    return Err(CompileError::LifetimeInversion {
                        provider: *identifier,
                        provider_lifetime: common.lifetime,
                        provider_declaration: common.source,
                        dependency,
                        dependency_lifetime,
                        dependency_declaration,
                    });
                }

                input.storage = match dependency_lifetime {
                    Lifetime::Singleton => InputStorage::Singleton,
                    Lifetime::Scoped => InputStorage::Scoped,
                    Lifetime::Transient => unreachable!(
                        "the scope compiler rejects transient providers before graph annotation"
                    ),
                };
            }
        }

        Ok(())
    }

    fn compile_selected(
        &mut self,
        identifier: ServiceIdentifier,
        selected: ProviderId,
    ) -> Result<(), CompileError> {
        match self.states.get(&identifier) {
            Some(VisitState::Done) => return Ok(()),
            Some(VisitState::Visiting) => {
                return Err(CompileError::Cycle {
                    chain: self.cycle_chain(identifier),
                });
            }
            None => {}
        }

        let provider = self.registry.provider(selected).clone();
        self.ensure_supported(identifier, &provider)?;

        self.states.insert(identifier, VisitState::Visiting);
        self.stack.push(identifier);

        let result = (|| {
            let mut positions = BTreeSet::new();
            let mut inputs = Vec::with_capacity(provider_dependencies(&provider).len());
            for request in provider_dependencies(&provider).iter().copied() {
                if !positions.insert(request.input_position) {
                    return Err(CompileError::InvalidInputLayout {
                        provider: identifier,
                        reason: "多个依赖占用了同一个构造输入位置",
                    });
                }
                inputs.push(self.compile_input(identifier, request)?);
            }

            self.providers
                .insert(identifier, CompiledProvider { provider, inputs });
            self.order.push(identifier);
            self.states.insert(identifier, VisitState::Done);
            Ok(())
        })();

        self.stack.pop();
        result
    }

    fn compile_input(
        &mut self,
        provider: ServiceIdentifier,
        request: DependencyRequest,
    ) -> Result<CompiledInput, CompileError> {
        match request.delivery {
            Delivery::Direct(prepare) => {
                let dependency = self.resolve_dependency(
                    provider,
                    request,
                    request.token,
                    request.provider_source,
                )?;
                Ok(CompiledInput {
                    position: request.input_position,
                    dependency,
                    storage: InputStorage::Absent,
                    prepare,
                })
            }
            Delivery::RequiresBinding => {
                if request.optional {
                    return Err(CompileError::InvalidInputLayout {
                        provider,
                        reason: "可选 trait 依赖必须携带缺席输入准备函数",
                    });
                }
                let binding = self.resolve_binding(provider, request)?;
                let concrete = binding
                    .key_policy
                    .concrete_identifier(request.token, binding.concrete_type);
                let dependency = self
                    .resolve_dependency(provider, request, concrete, ProviderSource::Registered)?
                    .expect("必选 trait 依赖不能解析为缺席输入");
                Ok(CompiledInput {
                    position: request.input_position,
                    dependency: Some(dependency),
                    storage: InputStorage::Absent,
                    prepare: binding.prepare_required,
                })
            }
            Delivery::RequiresBindingOrAbsent(absent_prepare) => {
                if !request.optional {
                    return Err(CompileError::InvalidInputLayout {
                        provider,
                        reason: "必选 trait 依赖不能使用可选缺席输入准备函数",
                    });
                }

                let Some(binding) = self.resolve_optional_binding(provider, request)? else {
                    return Ok(CompiledInput {
                        position: request.input_position,
                        dependency: None,
                        storage: InputStorage::Absent,
                        prepare: absent_prepare,
                    });
                };

                let concrete = binding
                    .key_policy
                    .concrete_identifier(request.token, binding.concrete_type);
                let dependency = self.resolve_dependency(
                    provider,
                    request,
                    concrete,
                    ProviderSource::Registered,
                )?;
                Ok(CompiledInput {
                    position: request.input_position,
                    dependency,
                    storage: InputStorage::Absent,
                    prepare: binding.prepare_optional,
                })
            }
        }
    }

    fn resolve_binding(
        &self,
        provider: ServiceIdentifier,
        request: DependencyRequest,
    ) -> Result<TraitBinding, CompileError> {
        self.registry
            .select_binding(request.token.service_type)
            .map_err(|candidates| CompileError::AmbiguousTraitBinding {
                provider,
                dependency: request.token,
                candidates,
            })?
            .ok_or(CompileError::MissingTraitBinding {
                provider,
                dependency: request.token,
            })
    }

    fn resolve_optional_binding(
        &self,
        provider: ServiceIdentifier,
        request: DependencyRequest,
    ) -> Result<Option<TraitBinding>, CompileError> {
        self.registry
            .select_binding(request.token.service_type)
            .map_err(|candidates| CompileError::AmbiguousTraitBinding {
                provider,
                dependency: request.token,
                candidates,
            })
    }

    /// 选择一个 exact token provider，并在需要时先物化闭合泛型 fallback。
    ///
    /// 显式注册永远先于 callback 被观察；这使 explicit closed-generic factory/class
    /// provider 能覆盖 `ProviderDefinition`，且 callback 不会产生额外副作用。
    fn resolve_dependency(
        &mut self,
        provider: ServiceIdentifier,
        request: DependencyRequest,
        target: ServiceIdentifier,
        source: ProviderSource,
    ) -> Result<Option<ServiceIdentifier>, CompileError> {
        let selected = self
            .registry
            .select_explicit(target)
            .map_err(|candidates| CompileError::AmbiguousDependency {
                provider,
                dependency: target,
                candidates,
            })?;

        let selected = match selected {
            Some(selected) => Some(selected),
            None => match source {
                ProviderSource::Registered => None,
                ProviderSource::Materialize(callback) => {
                    Some(self.registry.materialize(target, callback)?)
                }
            },
        };

        let Some(selected) = selected else {
            if request.optional {
                return Ok(None);
            }
            return Err(CompileError::MissingDependency {
                provider,
                dependency: target,
                label: request.label,
            });
        };

        self.compile_selected(target, selected)?;
        Ok(Some(target))
    }

    fn ensure_supported(
        &self,
        identifier: ServiceIdentifier,
        provider: &Provider,
    ) -> Result<(), CompileError> {
        let common = provider_common(provider);
        if common.lifetime != Lifetime::Singleton
            && !(self.lifetime_capability == LifetimeCapability::SingletonAndScoped
                && common.lifetime == Lifetime::Scoped)
        {
            return Err(CompileError::UnsupportedLifetime {
                provider: identifier,
                lifetime: common.lifetime,
                declaration: common.source,
            });
        }
        if self.activation_capability == ActivationCapability::SyncOnly && common.cleanup.is_some()
        {
            return Err(CompileError::UnsupportedCleanup {
                provider: identifier,
                declaration: common.source,
            });
        }
        if self.activation_capability == ActivationCapability::SyncOnly
            && matches!(provider, Provider::Factory(factory) if matches!(factory.invoker, FactoryInvoker::Async(_)))
        {
            return Err(CompileError::UnsupportedAsyncFactory {
                provider: identifier,
                declaration: common.source,
            });
        }
        Ok(())
    }

    fn cycle_chain(&self, repeated: ServiceIdentifier) -> Vec<ServiceIdentifier> {
        let start = self
            .stack
            .iter()
            .position(|identifier| *identifier == repeated)
            .expect("a visiting provider must be present in the compiler stack");
        let mut chain = self.stack[start..].to_vec();
        chain.push(repeated);
        chain
    }
}

/// 以 graph 的后序激活所有 provider，并把每项产物提交至同一个 Arena。
///
/// 任何错误都会丢弃局部 Arena；其 `Drop` 实现会按 commit 逆序析构已发布实例，所以
/// 这里不需要另行实现不安全的回滚逻辑。
fn activate(graph: &CompiledProviderGraph) -> Result<Arena, BuildError> {
    let mut arena = Arena::new();

    for identifier in &graph.order {
        let prepared = prepare_activation(graph, *identifier, &arena)?;
        commit_activation(&mut arena, activate_prepared_sync(prepared))?;
    }

    debug_assert!(arena.contains(graph.root));
    Ok(arena)
}

/// 一个已准备、但尚未调用 provider 的异步激活任务。
///
/// 它拥有构造输入与 provider payload，绝不借用 Arena；因此它可以安全地跨 await 存在，
/// 而 Arena 始终只由会话协调器在线程内访问。
struct PreparedActivation {
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
    cleanup: Option<crate::registration::provider::CleanupHook>,
    provider: Provider,
    context: ConstructionContext,
}

/// 一个完成的 provider 激活结果，尚未由协调器提交到 Arena。
struct ActivationOutcome {
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
    cleanup: Option<crate::registration::provider::CleanupHook>,
    result: Result<ErasedService, ActivationError>,
}

/// 准备一个已经由调度器判定为就绪的 provider。
///
/// 本函数在 await 边界之外执行：它可以短暂读取 Arena 以把稳定地址写入
/// `ConstructionContext`，但返回后不会保留 Arena 借用。
fn prepare_activation(
    graph: &CompiledProviderGraph,
    identifier: ServiceIdentifier,
    arena: &Arena,
) -> Result<PreparedActivation, BuildError> {
    prepare_activation_from(&graph.providers, identifier, |input| {
        input
            .dependency
            .and_then(|dependency| arena.lookup(dependency))
    })
}

/// 从已编译 provider 和明确的输入查找策略准备一个构造上下文。
///
/// 两种 runtime 都复用这个函数：Singleton-only 图从同一个 Arena 读取，而 scope 图
/// 根据 [`InputStorage`] 从父 Singleton Arena 或当前 Scoped Arena 读取。查找发生在
/// await 之前，返回的 [`PreparedActivation`] 不借用任何 Arena。
fn prepare_activation_from(
    providers: &BTreeMap<ServiceIdentifier, CompiledProvider>,
    identifier: ServiceIdentifier,
    mut lookup: impl FnMut(&CompiledInput) -> Option<ArenaServiceRef>,
) -> Result<PreparedActivation, BuildError> {
    let node = providers
        .get(&identifier)
        .expect("compiled activation schedule must reference an existing provider");
    let common = provider_common(&node.provider);
    let declaration = common.source;
    let mut context = ConstructionContext::new();

    for input in &node.inputs {
        let dependency = lookup(input);
        (input.prepare)(&mut context, input.position, dependency)
            .map_err(|error| BuildError::activation(identifier, declaration, error))?;
    }

    Ok(PreparedActivation {
        identifier,
        declaration,
        cleanup: common.cleanup,
        provider: node.provider.clone(),
        context,
    })
}

/// 以 scope 编译计划中记录的 Arena 来源准备一个 Scoped provider。
fn prepare_scoped_activation(
    plan: &CompiledScopePlan,
    identifier: ServiceIdentifier,
    singleton_arena: &Arena,
    scoped_arena: &Arena,
) -> Result<PreparedActivation, BuildError> {
    prepare_activation_from(&plan.providers, identifier, |input| {
        let dependency = input.dependency?;
        match input.storage {
            InputStorage::Absent => None,
            InputStorage::Singleton => singleton_arena.lookup(dependency),
            InputStorage::Scoped => scoped_arena.lookup(dependency),
        }
    })
}

/// 为 scope provider 构建阶段中需一次性发布的 Singleton 节点准备输入。
///
/// `CompiledScopePlan` 已在编译期阻止 Singleton 捕获 Scoped 服务，因此这里绝不会
/// 对当前 scope Arena 做回退查找。若这个不变量被 core 内部代码破坏，直接 panic 比
/// 在 runtime 静默改变生命周期边界更容易定位问题。
fn prepare_scope_singleton_activation(
    plan: &CompiledScopePlan,
    identifier: ServiceIdentifier,
    singleton_arena: &Arena,
) -> Result<PreparedActivation, BuildError> {
    prepare_activation_from(&plan.providers, identifier, |input| match input.storage {
        InputStorage::Absent => None,
        InputStorage::Singleton => input
            .dependency
            .and_then(|dependency| singleton_arena.lookup(dependency)),
        InputStorage::Scoped => {
            unreachable!("the scope compiler must reject Singleton-to-Scoped dependencies")
        }
    })
}

/// 不跨 await 地调用一个已准备 provider。
fn activate_prepared_sync(prepared: PreparedActivation) -> ActivationOutcome {
    let PreparedActivation {
        identifier,
        declaration,
        cleanup,
        provider,
        context,
    } = prepared;

    let result = match provider {
        Provider::Class(class) => (class.constructor)(context),
        Provider::Factory(factory) => {
            let frame = FactoryActivationFrame::new();
            match factory.invoker {
                FactoryInvoker::Sync(constructor) => {
                    constructor(FactoryConstructionContext::from_bound(context, &frame))
                }
                FactoryInvoker::Async(_) => unreachable!(
                    "a synchronous activation entry must reject async factories before invocation"
                ),
            }
        }
    };

    ActivationOutcome {
        identifier,
        declaration,
        cleanup,
        result,
    }
}

/// 提交一个已经完成的同步 provider 构造结果。
///
/// 局部 `Arena` 在本函数的调用方拥有；因此任意 activation 或 commit 错误都会让
/// 调用方离开其作用域，并由 Arena 的 Drop 按提交逆序回滚已发布的服务。
fn commit_activation(arena: &mut Arena, outcome: ActivationOutcome) -> Result<(), BuildError> {
    let ActivationOutcome {
        identifier,
        declaration,
        cleanup,
        result,
    } = outcome;
    let service = result.map_err(|error| BuildError::activation(identifier, declaration, error))?;

    arena
        .commit_with_cleanup(identifier, service, cleanup)
        .map_err(|error| BuildError::arena(identifier, declaration, error))
}

/// 验证一个同步调用点即将执行的节点中没有 async factory 或 cleanup hook。
///
/// 异步 scope provider 可以先以 async 能力编译完整链；之后用户仍可在该 provider 上
/// 调用 `create_scope()`。这个额外检查让同步 scope 只拒绝自己的 Scoped
/// 子图，而不会重新拒绝已成功发布的 async factory 或 cleanup Singleton。
fn ensure_sync_schedule(
    providers: &BTreeMap<ServiceIdentifier, CompiledProvider>,
    schedule: &[ServiceIdentifier],
) -> Result<(), BuildError> {
    for identifier in schedule {
        let node = providers
            .get(identifier)
            .expect("compiled activation schedule must reference an existing provider");
        let common = provider_common(&node.provider);
        if common.cleanup.is_some() {
            return Err(CompileError::UnsupportedCleanup {
                provider: *identifier,
                declaration: common.source,
            }
            .into());
        }
        if matches!(node.provider, Provider::Factory(ref factory) if matches!(factory.invoker, FactoryInvoker::Async(_)))
        {
            return Err(CompileError::UnsupportedAsyncFactory {
                provider: *identifier,
                declaration: common.source,
            }
            .into());
        }
    }
    Ok(())
}

/// 以固定后序顺序构建双根计划中的 Singleton 闭包。
fn activate_scope_singletons(plan: &CompiledScopePlan) -> Result<Arena, BuildError> {
    ensure_sync_schedule(&plan.providers, &plan.singleton_order)?;

    let mut arena = Arena::new();
    for identifier in &plan.singleton_order {
        let prepared = prepare_scope_singleton_activation(plan, *identifier, &arena)?;
        commit_activation(&mut arena, activate_prepared_sync(prepared))?;
    }

    debug_assert!(arena.contains(plan.app_root));
    Ok(arena)
}

/// 为一次新建 scope eager 构建其 Scoped 闭包。
///
/// 这里的 Arena 是局部变量：失败不会触及父 Singleton Arena，只会由自身 Drop 回滚
/// 已经提交的 Scoped 实例。
fn activate_scoped(plan: &CompiledScopePlan, singleton_arena: &Arena) -> Result<Arena, BuildError> {
    ensure_sync_schedule(&plan.providers, &plan.scoped_order)?;

    let mut scoped_arena = Arena::new();
    for identifier in &plan.scoped_order {
        let prepared =
            prepare_scoped_activation(plan, *identifier, singleton_arena, &scoped_arena)?;
        commit_activation(&mut scoped_arena, activate_prepared_sync(prepared))?;
    }

    debug_assert!(scoped_arena.contains(plan.scope_root));
    Ok(scoped_arena)
}

fn provider_identifier(provider: &Provider) -> ServiceIdentifier {
    match provider {
        Provider::Class(provider) => provider.provide,
        Provider::Factory(provider) => provider.provide,
    }
}

fn provider_common(provider: &Provider) -> ProviderCommon {
    match provider {
        Provider::Class(provider) => provider.common,
        Provider::Factory(provider) => provider.common,
    }
}

fn provider_dependencies(provider: &Provider) -> &[DependencyRequest] {
    match provider {
        Provider::Class(provider) => &provider.dependencies,
        Provider::Factory(provider) => &provider.dependencies,
    }
}

// ---------------------------------------------------------------------------
// v4 occurrence-aware compilation
// ---------------------------------------------------------------------------

/// 一次实际实例化的私有身份。
///
/// `ServiceIdentifier` 仍是 provider 选择与泛型蓝图缓存的身份；此 ID 只区分同一
/// token 在不同 transient 注入边上的实例，绝不暴露为 service locator API。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ActivationNodeId(usize);

/// 静态 Scope 链中从外到内的层编号。
///
/// 该编号只存在于编译计划与运行时 Arena 选择中，不是公开的服务身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct ScopeFrameId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationPhase {
    Singleton,
    Scope(ScopeFrameId),
}

/// transient 子树实际附着的持久生命周期域。
///
/// 这不是 provider 声明的生命周期：Transient 沿着消费边继承持久 owner，故它不能
/// 借由中间节点把 Scoped 指针保存进 Singleton Arena。
#[derive(Debug, Clone, Copy)]
enum EffectiveOwner {
    Singleton {
        identifier: ServiceIdentifier,
        declaration: ServiceSource,
    },
    Scope {
        frame: ScopeFrameId,
        identifier: ServiceIdentifier,
        declaration: ServiceSource,
    },
}

impl EffectiveOwner {
    fn phase(self) -> ActivationPhase {
        match self {
            Self::Singleton { .. } => ActivationPhase::Singleton,
            Self::Scope { frame, .. } => ActivationPhase::Scope(frame),
        }
    }

    fn lifetime(self) -> Lifetime {
        match self {
            Self::Singleton { .. } => Lifetime::Singleton,
            Self::Scope { .. } => Lifetime::Scoped,
        }
    }

    fn identifier(self) -> ServiceIdentifier {
        match self {
            Self::Singleton { identifier, .. } | Self::Scope { identifier, .. } => identifier,
        }
    }

    fn declaration(self) -> ServiceSource {
        match self {
            Self::Singleton { declaration, .. } | Self::Scope { declaration, .. } => declaration,
        }
    }

    fn frame(self) -> Option<ScopeFrameId> {
        match self {
            Self::Singleton { .. } => None,
            Self::Scope { frame, .. } => Some(frame),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FactoryTemporaryOwner {
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
}

/// 一个已解析输入从哪个持有区域读取。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationInputStorage {
    Absent,
    Singleton,
    Scoped(ScopeFrameId),
    /// class consumer 的字段 transient；会被移交给 consumer 的 child arenas。
    TransientChild,
    /// factory 参数 transient；仅由本次 activation frame 暂时持有。
    TransientFactoryParameter,
}

#[derive(Clone, Copy)]
struct ActivationInput {
    position: crate::construction::InputPosition,
    dependency: Option<ActivationNodeId>,
    storage: ActivationInputStorage,
    prepare: PrepareInput,
}

#[derive(Clone)]
struct ActivationNode {
    identifier: ServiceIdentifier,
    provider: Provider,
    inputs: Vec<ActivationInput>,
    lifetime: Lifetime,
    phase: ActivationPhase,
    owner: EffectiveOwner,
}

struct ActivationGraph {
    root: ActivationNodeId,
    nodes: Vec<ActivationNode>,
    order: Vec<ActivationNodeId>,
}

impl ActivationGraph {
    fn node(&self, id: ActivationNodeId) -> &ActivationNode {
        &self.nodes[id.0]
    }
}

struct ActivationRootGraph {
    graph: ActivationGraph,
}

struct ActivationScopePlan {
    app_root: ActivationNodeId,
    graph: ActivationGraph,
    singleton_order: Vec<ActivationNodeId>,
    scope_levels: Vec<ScopeLevelPlan>,
}

/// 一层静态 Scope 的 root、激活顺序及诊断上下文。
struct ScopeLevelPlan {
    frame: ScopeFrameId,
    root: ActivationNodeId,
    root_identifier: ServiceIdentifier,
    root_declaration: ServiceSource,
    order: Vec<ActivationNodeId>,
}

/// Scope 链预扫描得到的 root 声明。
///
/// 在 DFS 前保留它，使祖先闭包试图捕获后代 root 时能给出生命周期诊断，而不是先把
/// 后代服务悄悄归属到祖先层，再在处理 child root 时才失败。
#[derive(Clone, Copy)]
struct DeclaredScopeRoot {
    frame: ScopeFrameId,
    declaration: ServiceSource,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ActivationCapabilityV4 {
    SyncOnly,
    AsyncFactoriesAndCleanup,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ScopeCapabilityV4 {
    SingletonOnly,
    SingletonAndScoped,
}

/// v4 compiler：持久节点按 token canonicalize，Transient 则按消费边产生新节点。
struct ActivationCompiler {
    registry: ProviderRegistry,
    activation_capability: ActivationCapabilityV4,
    scope_capability: ScopeCapabilityV4,
    persistent: BTreeMap<ServiceIdentifier, ActivationNodeId>,
    states: BTreeMap<ActivationNodeId, VisitState>,
    stack: Vec<ActivationNodeId>,
    nodes: Vec<ActivationNode>,
    order: Vec<ActivationNodeId>,
    declared_scope_roots: BTreeMap<ServiceIdentifier, DeclaredScopeRoot>,
}

impl ActivationCompiler {
    fn new(registry: ProviderRegistry) -> Self {
        Self {
            registry,
            activation_capability: ActivationCapabilityV4::SyncOnly,
            scope_capability: ScopeCapabilityV4::SingletonOnly,
            persistent: BTreeMap::new(),
            states: BTreeMap::new(),
            stack: Vec::new(),
            nodes: Vec::new(),
            order: Vec::new(),
            declared_scope_roots: BTreeMap::new(),
        }
    }

    fn compile_root<Root>(mut self) -> Result<ActivationRootGraph, CompileError>
    where
        Root: Injectable,
    {
        let (root, selected) = self.select_root::<Root>()?;
        let common = provider_common(self.registry.provider(selected));
        self.ensure_root_lifetime(root, common, Lifetime::Singleton)?;
        let root = self.compile_selected(
            root,
            selected,
            EffectiveOwner::Singleton {
                identifier: root,
                declaration: common.source,
            },
            None,
            None,
        )?;

        Ok(ActivationRootGraph {
            graph: ActivationGraph {
                root,
                nodes: self.nodes,
                order: self.order,
            },
        })
    }

    fn compile_async_root<Root>(mut self) -> Result<ActivationRootGraph, CompileError>
    where
        Root: Injectable,
    {
        self.activation_capability = ActivationCapabilityV4::AsyncFactoriesAndCleanup;
        self.compile_root::<Root>()
    }

    fn compile_scope_plan<AppRoot, ScopeRoot>(mut self) -> Result<ActivationScopePlan, CompileError>
    where
        AppRoot: Injectable,
        ScopeRoot: Injectable,
    {
        self.scope_capability = ScopeCapabilityV4::SingletonAndScoped;
        self.compile_scope_identifiers::<AppRoot>(vec![ServiceIdentifier::from(
            ServiceType::create::<ScopeRoot>(),
        )])
    }

    fn compile_async_scope_plan<AppRoot, ScopeRoot>(
        mut self,
    ) -> Result<ActivationScopePlan, CompileError>
    where
        AppRoot: Injectable,
        ScopeRoot: Injectable,
    {
        self.activation_capability = ActivationCapabilityV4::AsyncFactoriesAndCleanup;
        self.scope_capability = ScopeCapabilityV4::SingletonAndScoped;
        self.compile_scope_identifiers::<AppRoot>(vec![ServiceIdentifier::from(
            ServiceType::create::<ScopeRoot>(),
        )])
    }

    fn compile_scope_chain<AppRoot, Chain>(mut self) -> Result<ActivationScopePlan, CompileError>
    where
        AppRoot: Injectable,
        Chain: ScopeChain,
    {
        self.scope_capability = ScopeCapabilityV4::SingletonAndScoped;
        let mut scope_roots = Vec::new();
        Chain::collect_scope_roots(&mut scope_roots);
        self.compile_scope_identifiers::<AppRoot>(scope_roots)
    }

    fn compile_async_scope_chain<AppRoot, Chain>(
        mut self,
    ) -> Result<ActivationScopePlan, CompileError>
    where
        AppRoot: Injectable,
        Chain: ScopeChain,
    {
        self.activation_capability = ActivationCapabilityV4::AsyncFactoriesAndCleanup;
        self.scope_capability = ScopeCapabilityV4::SingletonAndScoped;
        let mut scope_roots = Vec::new();
        Chain::collect_scope_roots(&mut scope_roots);
        self.compile_scope_identifiers::<AppRoot>(scope_roots)
    }

    fn compile_scope_identifiers<AppRoot>(
        mut self,
        scope_identifiers: Vec<ServiceIdentifier>,
    ) -> Result<ActivationScopePlan, CompileError>
    where
        AppRoot: Injectable,
    {
        debug_assert!(
            !scope_identifiers.is_empty(),
            "a static ScopeChain always has a ScopeLayer head"
        );

        // 先预扫描整条链。这样祖先闭包请求一个声明为后代 root 的 Scoped token 时，
        // 能在 DFS 期间给出 ScopeRootOwnedByAncestor，而不会错误地把它归属到祖先层。
        let mut selected_scope_roots = Vec::with_capacity(scope_identifiers.len());
        for (index, scope_identifier) in scope_identifiers.iter().copied().enumerate() {
            let frame = ScopeFrameId(index);
            let selected = self.select_root_identifier(scope_identifier)?;
            let common = provider_common(self.registry.provider(selected));
            self.ensure_root_lifetime(scope_identifier, common, Lifetime::Scoped)?;

            if let Some(ancestor) = self.declared_scope_roots.insert(
                scope_identifier,
                DeclaredScopeRoot {
                    frame,
                    declaration: common.source,
                },
            ) {
                let ancestor_root = scope_identifiers[ancestor.frame.0];
                return Err(CompileError::ScopeRootOwnedByAncestor {
                    root: scope_identifier,
                    declaration: common.source,
                    ancestor_root,
                    ancestor_declaration: ancestor.declaration,
                });
            }
            selected_scope_roots.push((frame, scope_identifier, selected, common));
        }

        let (app_identifier, app_selected) = self.select_root::<AppRoot>()?;
        let app_common = provider_common(self.registry.provider(app_selected));
        self.ensure_root_lifetime(app_identifier, app_common, Lifetime::Singleton)?;
        let app_root = self.compile_selected(
            app_identifier,
            app_selected,
            EffectiveOwner::Singleton {
                identifier: app_identifier,
                declaration: app_common.source,
            },
            None,
            None,
        )?;

        let mut scope_levels: Vec<ScopeLevelPlan> = Vec::with_capacity(selected_scope_roots.len());
        for (frame, scope_identifier, scope_selected, scope_common) in selected_scope_roots {
            let scope_root = self.compile_selected(
                scope_identifier,
                scope_selected,
                EffectiveOwner::Scope {
                    frame,
                    identifier: scope_identifier,
                    declaration: scope_common.source,
                },
                None,
                None,
            )?;

            let actual_frame = match self.nodes[scope_root.0].phase {
                ActivationPhase::Scope(actual_frame) => actual_frame,
                ActivationPhase::Singleton => unreachable!(
                    "a root validated as Scoped cannot be represented by a Singleton node"
                ),
            };
            if actual_frame != frame {
                let ancestor = scope_levels
                    .get(actual_frame.0)
                    .expect("a reused ScopeRoot can only be owned by an earlier compiled frame");
                return Err(CompileError::ScopeRootOwnedByAncestor {
                    root: scope_identifier,
                    declaration: scope_common.source,
                    ancestor_root: ancestor.root_identifier,
                    ancestor_declaration: ancestor.root_declaration,
                });
            }
            scope_levels.push(ScopeLevelPlan {
                frame,
                root: scope_root,
                root_identifier: scope_identifier,
                root_declaration: scope_common.source,
                order: Vec::new(),
            });
        }

        let singleton_order = self
            .order
            .iter()
            .copied()
            .filter(|id| self.nodes[id.0].phase == ActivationPhase::Singleton)
            .collect();
        for scope_level in &mut scope_levels {
            scope_level.order = self
                .order
                .iter()
                .copied()
                .filter(|id| self.nodes[id.0].phase == ActivationPhase::Scope(scope_level.frame))
                .collect();
        }

        Ok(ActivationScopePlan {
            app_root,
            graph: ActivationGraph {
                root: app_root,
                nodes: self.nodes,
                order: self.order,
            },
            singleton_order,
            scope_levels,
        })
    }

    fn select_root<Root>(&self) -> Result<(ServiceIdentifier, ProviderId), CompileError>
    where
        Root: Injectable,
    {
        let root = ServiceIdentifier::from(ServiceType::create::<Root>());
        self.select_root_identifier(root)
            .map(|provider| (root, provider))
    }

    fn select_root_identifier(&self, root: ServiceIdentifier) -> Result<ProviderId, CompileError> {
        let Some(provider) = self.registry.select_explicit(root).map_err(|candidates| {
            CompileError::AmbiguousDependency {
                provider: root,
                dependency: root,
                candidates,
            }
        })?
        else {
            return Err(CompileError::MissingRoot { root });
        };
        Ok(provider)
    }

    fn ensure_root_lifetime(
        &self,
        root: ServiceIdentifier,
        common: ProviderCommon,
        expected: Lifetime,
    ) -> Result<(), CompileError> {
        if common.lifetime != expected {
            return Err(CompileError::InvalidRootLifetime {
                root,
                expected,
                actual: common.lifetime,
                declaration: common.source,
            });
        }
        Ok(())
    }

    fn compile_selected(
        &mut self,
        identifier: ServiceIdentifier,
        selected: ProviderId,
        requested_owner: EffectiveOwner,
        parent: Option<ActivationNodeId>,
        factory_temporary: Option<FactoryTemporaryOwner>,
    ) -> Result<ActivationNodeId, CompileError> {
        let provider = self.registry.provider(selected).clone();
        let common = provider_common(&provider);
        self.ensure_owner_compatibility(identifier, common, requested_owner, parent)?;
        self.ensure_supported(identifier, &provider)?;

        if common.lifetime != Lifetime::Transient {
            if let Some(existing) = self.persistent.get(&identifier).copied() {
                return match self.states.get(&existing) {
                    Some(VisitState::Done) => Ok(existing),
                    Some(VisitState::Visiting) => Err(CompileError::Cycle {
                        chain: self.cycle_chain(existing),
                    }),
                    None => unreachable!("persistent activation node must have a visit state"),
                };
            }
        } else if let Some(repeated) = self
            .stack
            .iter()
            .copied()
            .find(|node| self.nodes[node.0].identifier == identifier)
        {
            return Err(CompileError::Cycle {
                chain: self.cycle_chain(repeated),
            });
        }

        if common.lifetime == Lifetime::Transient
            && common.cleanup.is_some()
            && let Some(temporary) = factory_temporary
        {
            return Err(CompileError::TransientFactoryParameterCleanup {
                factory: temporary.identifier,
                factory_declaration: temporary.declaration,
                provider: identifier,
                declaration: common.source,
                chain: self.chain_with(identifier),
            });
        }

        let owner = match common.lifetime {
            Lifetime::Singleton => EffectiveOwner::Singleton {
                identifier,
                declaration: common.source,
            },
            Lifetime::Scoped => EffectiveOwner::Scope {
                frame: requested_owner
                    .frame()
                    .expect("a Scoped provider can only be compiled from a Scoped owner"),
                identifier,
                declaration: common.source,
            },
            Lifetime::Transient => requested_owner,
        };
        let phase = match common.lifetime {
            Lifetime::Singleton => ActivationPhase::Singleton,
            Lifetime::Scoped => owner.phase(),
            Lifetime::Transient => owner.phase(),
        };
        let id = ActivationNodeId(self.nodes.len());
        self.nodes.push(ActivationNode {
            identifier,
            provider: provider.clone(),
            inputs: Vec::new(),
            lifetime: common.lifetime,
            phase,
            owner,
        });
        if common.lifetime != Lifetime::Transient {
            let previous = self.persistent.insert(identifier, id);
            debug_assert!(previous.is_none());
        }
        self.states.insert(id, VisitState::Visiting);
        self.stack.push(id);

        let result = (|| {
            let mut positions = BTreeSet::new();
            let mut inputs = Vec::with_capacity(provider_dependencies(&provider).len());
            let parent_is_factory = matches!(provider, Provider::Factory(_));
            for request in provider_dependencies(&provider).iter().copied() {
                if !positions.insert(request.input_position) {
                    return Err(CompileError::InvalidInputLayout {
                        provider: identifier,
                        reason: "多个依赖占用了同一个构造输入位置",
                    });
                }
                inputs.push(self.compile_input(
                    id,
                    identifier,
                    request,
                    parent_is_factory,
                    factory_temporary,
                )?);
            }
            self.nodes[id.0].inputs = inputs;
            self.order.push(id);
            self.states.insert(id, VisitState::Done);
            Ok(id)
        })();

        self.stack.pop();
        if result.is_err() && common.lifetime != Lifetime::Transient {
            self.persistent.remove(&identifier);
        }
        result
    }

    fn compile_input(
        &mut self,
        parent: ActivationNodeId,
        provider: ServiceIdentifier,
        request: DependencyRequest,
        parent_is_factory: bool,
        inherited_temporary: Option<FactoryTemporaryOwner>,
    ) -> Result<ActivationInput, CompileError> {
        match request.delivery {
            Delivery::Direct(prepare) => {
                let dependency = self.resolve_dependency(
                    parent,
                    provider,
                    request,
                    request.token,
                    request.provider_source,
                    parent_is_factory,
                    inherited_temporary,
                )?;
                Ok(ActivationInput {
                    position: request.input_position,
                    dependency,
                    storage: self.input_storage(dependency, parent_is_factory),
                    prepare,
                })
            }
            Delivery::RequiresBinding => {
                if request.optional {
                    return Err(CompileError::InvalidInputLayout {
                        provider,
                        reason: "可选 trait 依赖必须携带缺席输入准备函数",
                    });
                }
                let binding = self.resolve_binding(provider, request)?;
                let concrete = binding
                    .key_policy
                    .concrete_identifier(request.token, binding.concrete_type);
                let dependency = self
                    .resolve_dependency(
                        parent,
                        provider,
                        request,
                        concrete,
                        ProviderSource::Registered,
                        parent_is_factory,
                        inherited_temporary,
                    )?
                    .expect("required trait dependency cannot be absent");
                Ok(ActivationInput {
                    position: request.input_position,
                    dependency: Some(dependency),
                    storage: self.input_storage(Some(dependency), parent_is_factory),
                    prepare: binding.prepare_required,
                })
            }
            Delivery::RequiresBindingOrAbsent(absent_prepare) => {
                if !request.optional {
                    return Err(CompileError::InvalidInputLayout {
                        provider,
                        reason: "必选 trait 依赖不能使用可选缺席输入准备函数",
                    });
                }
                let Some(binding) = self.resolve_optional_binding(provider, request)? else {
                    return Ok(ActivationInput {
                        position: request.input_position,
                        dependency: None,
                        storage: ActivationInputStorage::Absent,
                        prepare: absent_prepare,
                    });
                };
                let concrete = binding
                    .key_policy
                    .concrete_identifier(request.token, binding.concrete_type);
                let dependency = self.resolve_dependency(
                    parent,
                    provider,
                    request,
                    concrete,
                    ProviderSource::Registered,
                    parent_is_factory,
                    inherited_temporary,
                )?;
                Ok(ActivationInput {
                    position: request.input_position,
                    dependency,
                    storage: self.input_storage(dependency, parent_is_factory),
                    prepare: binding.prepare_optional,
                })
            }
        }
    }

    fn input_storage(
        &self,
        dependency: Option<ActivationNodeId>,
        parent_is_factory: bool,
    ) -> ActivationInputStorage {
        let Some(dependency) = dependency else {
            return ActivationInputStorage::Absent;
        };
        match self.nodes[dependency.0].lifetime {
            Lifetime::Singleton => ActivationInputStorage::Singleton,
            Lifetime::Scoped => match self.nodes[dependency.0].phase {
                ActivationPhase::Scope(frame) => ActivationInputStorage::Scoped(frame),
                ActivationPhase::Singleton => unreachable!(
                    "a provider declared Scoped cannot be activated in the Singleton phase"
                ),
            },
            Lifetime::Transient if parent_is_factory => {
                ActivationInputStorage::TransientFactoryParameter
            }
            Lifetime::Transient => ActivationInputStorage::TransientChild,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_dependency(
        &mut self,
        parent: ActivationNodeId,
        provider: ServiceIdentifier,
        request: DependencyRequest,
        target: ServiceIdentifier,
        source: ProviderSource,
        parent_is_factory: bool,
        inherited_temporary: Option<FactoryTemporaryOwner>,
    ) -> Result<Option<ActivationNodeId>, CompileError> {
        let selected = self
            .registry
            .select_explicit(target)
            .map_err(|candidates| CompileError::AmbiguousDependency {
                provider,
                dependency: target,
                candidates,
            })?;
        let selected = match selected {
            Some(selected) => Some(selected),
            None => match source {
                ProviderSource::Registered => None,
                ProviderSource::Materialize(callback) => {
                    Some(self.registry.materialize(target, callback)?)
                }
            },
        };
        let Some(selected) = selected else {
            if request.optional {
                return Ok(None);
            }
            return Err(CompileError::MissingDependency {
                provider,
                dependency: target,
                label: request.label,
            });
        };

        let common = provider_common(self.registry.provider(selected));
        let parent_owner = self.nodes[parent.0].owner;
        let temporary = if common.lifetime == Lifetime::Transient {
            inherited_temporary.or_else(|| {
                parent_is_factory.then_some(FactoryTemporaryOwner {
                    identifier: provider,
                    declaration: provider_common(&self.nodes[parent.0].provider).source,
                })
            })
        } else {
            None
        };
        self.compile_selected(target, selected, parent_owner, Some(parent), temporary)
            .map(Some)
    }

    fn ensure_owner_compatibility(
        &self,
        dependency: ServiceIdentifier,
        dependency_common: ProviderCommon,
        owner: EffectiveOwner,
        parent: Option<ActivationNodeId>,
    ) -> Result<(), CompileError> {
        if dependency_common.lifetime != Lifetime::Scoped {
            return Ok(());
        }

        if let Some(owner_frame) = owner.frame() {
            if let Some(declared_dependency) = self.declared_scope_roots.get(&dependency)
                && declared_dependency.frame > owner_frame
            {
                let (ancestor_root, ancestor) = self
                    .declared_scope_roots
                    .iter()
                    .find(|(_, declared)| declared.frame == owner_frame)
                    .expect("every Scoped owner frame must originate from a declared ScopeRoot");
                if let Some(parent) = parent {
                    let parent_node = &self.nodes[parent.0];
                    if parent_node.identifier != *ancestor_root {
                        return Err(CompileError::ScopeLifetimeInversion {
                            provider: parent_node.identifier,
                            provider_declaration: provider_common(&parent_node.provider).source,
                            provider_scope_root: *ancestor_root,
                            dependency,
                            dependency_declaration: dependency_common.source,
                            dependency_scope_root: dependency,
                        });
                    }
                }
                // The target is itself a root declared for a later frame.  If the
                // ancestor closure were allowed to compile it, persistent-node
                // de-duplication would make the later root reuse the ancestor
                // Arena.  Diagnose that ownership violation directly rather than
                // allowing the generic lifetime check to hide it.
                return Err(CompileError::ScopeRootOwnedByAncestor {
                    root: dependency,
                    declaration: dependency_common.source,
                    ancestor_root: *ancestor_root,
                    ancestor_declaration: ancestor.declaration,
                });
            }
            return Ok(());
        }

        let Some(parent) = parent else {
            return Err(CompileError::InvalidRootLifetime {
                root: dependency,
                expected: Lifetime::Singleton,
                actual: Lifetime::Scoped,
                declaration: dependency_common.source,
            });
        };
        let parent_node = &self.nodes[parent.0];
        if parent_node.lifetime == Lifetime::Singleton {
            return Err(CompileError::LifetimeInversion {
                provider: parent_node.identifier,
                provider_lifetime: Lifetime::Singleton,
                provider_declaration: provider_common(&parent_node.provider).source,
                dependency,
                dependency_lifetime: Lifetime::Scoped,
                dependency_declaration: dependency_common.source,
            });
        }
        Err(CompileError::TransientLifetimeInversion {
            owner: owner.identifier(),
            owner_lifetime: owner.lifetime(),
            owner_declaration: owner.declaration(),
            dependency,
            dependency_declaration: dependency_common.source,
            chain: self.chain_with(dependency),
        })
    }

    fn ensure_supported(
        &self,
        identifier: ServiceIdentifier,
        provider: &Provider,
    ) -> Result<(), CompileError> {
        let common = provider_common(provider);
        if common.lifetime == Lifetime::Scoped
            && self.scope_capability != ScopeCapabilityV4::SingletonAndScoped
        {
            return Err(CompileError::UnsupportedLifetime {
                provider: identifier,
                lifetime: common.lifetime,
                declaration: common.source,
            });
        }
        if self.activation_capability == ActivationCapabilityV4::SyncOnly
            && common.cleanup.is_some()
        {
            return Err(CompileError::UnsupportedCleanup {
                provider: identifier,
                declaration: common.source,
            });
        }
        if self.activation_capability == ActivationCapabilityV4::SyncOnly
            && matches!(provider, Provider::Factory(factory) if matches!(factory.invoker, FactoryInvoker::Async(_)))
        {
            return Err(CompileError::UnsupportedAsyncFactory {
                provider: identifier,
                declaration: common.source,
            });
        }
        Ok(())
    }

    fn resolve_binding(
        &self,
        provider: ServiceIdentifier,
        request: DependencyRequest,
    ) -> Result<TraitBinding, CompileError> {
        self.registry
            .select_binding(request.token.service_type)
            .map_err(|candidates| CompileError::AmbiguousTraitBinding {
                provider,
                dependency: request.token,
                candidates,
            })?
            .ok_or(CompileError::MissingTraitBinding {
                provider,
                dependency: request.token,
            })
    }

    fn resolve_optional_binding(
        &self,
        provider: ServiceIdentifier,
        request: DependencyRequest,
    ) -> Result<Option<TraitBinding>, CompileError> {
        self.registry
            .select_binding(request.token.service_type)
            .map_err(|candidates| CompileError::AmbiguousTraitBinding {
                provider,
                dependency: request.token,
                candidates,
            })
    }

    fn chain_with(&self, last: ServiceIdentifier) -> Vec<ServiceIdentifier> {
        let mut chain = self
            .stack
            .iter()
            .map(|id| self.nodes[id.0].identifier)
            .collect::<Vec<_>>();
        chain.push(last);
        chain
    }

    fn cycle_chain(&self, repeated: ActivationNodeId) -> Vec<ServiceIdentifier> {
        let start = self
            .stack
            .iter()
            .position(|id| *id == repeated)
            .or_else(|| {
                self.stack
                    .iter()
                    .position(|id| self.nodes[id.0].identifier == self.nodes[repeated.0].identifier)
            })
            .expect("a visiting activation node must be in the compiler stack");
        let mut chain = self.stack[start..]
            .iter()
            .map(|id| self.nodes[id.0].identifier)
            .collect::<Vec<_>>();
        chain.push(self.nodes[repeated.0].identifier);
        chain
    }
}

// ---------------------------------------------------------------------------
// v4 occurrence-aware activation
// ---------------------------------------------------------------------------

/// 已完成、尚未移交给直接消费者的 transient instances。
///
/// 每个 entry 都是只含一个根服务的独立 Arena，故相同 ServiceIdentifier 的两个
/// transient occurrence 不会发生 Arena token 冲突。子 transient 在其直接消费者
/// 提交时被移动进该 consumer 的 DropEntry。
#[derive(Default)]
struct TransientStore {
    entries: BTreeMap<ActivationNodeId, Arena>,
}

impl TransientStore {
    fn lookup(
        &self,
        id: ActivationNodeId,
        identifier: ServiceIdentifier,
    ) -> Option<ArenaServiceRef> {
        self.entries
            .get(&id)
            .and_then(|arena| arena.lookup(identifier))
    }

    fn insert(&mut self, id: ActivationNodeId, arena: Arena) {
        let previous = self.entries.insert(id, arena);
        debug_assert!(previous.is_none(), "each transient occurrence commits once");
    }

    fn take_direct_inputs(&mut self, node: &ActivationNode) -> Vec<Arena> {
        let mut children = Vec::new();
        for input in &node.inputs {
            if !matches!(
                input.storage,
                ActivationInputStorage::TransientChild
                    | ActivationInputStorage::TransientFactoryParameter
            ) {
                continue;
            }
            let dependency = input
                .dependency
                .expect("a transient input must have a resolved activation node");
            let arena = self.entries.remove(&dependency).expect(
                "the scheduler must commit each direct transient input before its consumer",
            );
            children.push(arena);
        }
        children
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn arenas(&self) -> impl Iterator<Item = &Arena> {
        self.entries.values()
    }
}

enum PreparedTransientOwnership {
    PersistentChildren(Vec<Arena>),
    FactoryTemporary(Vec<Arena>),
}

struct V4PreparedActivation {
    id: ActivationNodeId,
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
    cleanup: Option<crate::registration::provider::CleanupHook>,
    provider: Provider,
    context: ConstructionContext,
    transient_ownership: PreparedTransientOwnership,
}

struct V4ActivationOutcome {
    id: ActivationNodeId,
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
    cleanup: Option<crate::registration::provider::CleanupHook>,
    result: Result<ErasedService, ActivationError>,
    children: Vec<Arena>,
}

/// Tokio worker 可移动的调用负载。
///
/// 它不携带 `Arena` 或 transient child ownership；所有原始 `Inject` 指针都由
/// `leases` 覆盖，直到 worker 的 invocation/frame/output 被销毁。
struct V4WorkerJob {
    id: ActivationNodeId,
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
    cleanup: Option<crate::registration::provider::CleanupHook>,
    provider: Provider,
    context: ConstructionContext,
    leases: ArenaLeaseSet,
}

/// worker 完成后的值。class 输出可能已经把 `Inject` 移入 service，故其 outcome 必须
/// 在 coordinator commit 或普通 drop 前继续持有 lease。factory 参数 token 受 frame
/// 限制，frame/future 销毁后不会出现在该 outcome 中。
enum V4WorkerOutcome {
    Class {
        id: ActivationNodeId,
        identifier: ServiceIdentifier,
        declaration: ServiceSource,
        cleanup: Option<crate::registration::provider::CleanupHook>,
        result: Result<ErasedService, ActivationError>,
        leases: ArenaLeaseSet,
    },
    Factory {
        id: ActivationNodeId,
        identifier: ServiceIdentifier,
        declaration: ServiceSource,
        cleanup: Option<crate::registration::provider::CleanupHook>,
        result: Result<ErasedService, ActivationError>,
    },
}

impl V4WorkerOutcome {
    fn id(&self) -> ActivationNodeId {
        match self {
            Self::Class { id, .. } | Self::Factory { id, .. } => *id,
        }
    }
}

/// `workers` 必须先于 coordinator 持有的 transient ownership 与主 Arena 析构。Tokio
/// abort 不会等待 worker 实际析构，因此 backing lease 进一步覆盖这段异步收口窗口。
struct V4AsyncActivationSession {
    workers: JoinSet<V4WorkerOutcome>,
    pending: BTreeMap<ActivationNodeId, PreparedTransientOwnership>,
    transients: TransientStore,
    arena: Arena,
}

impl V4AsyncActivationSession {
    fn new() -> Self {
        Self {
            workers: JoinSet::new(),
            pending: BTreeMap::new(),
            transients: TransientStore::default(),
            arena: Arena::new(),
        }
    }

    fn into_arena(self) -> Arena {
        let Self {
            workers,
            pending,
            transients,
            arena,
        } = self;
        debug_assert!(workers.is_empty());
        debug_assert!(pending.is_empty());
        debug_assert!(transients.is_empty());
        drop(workers);
        drop(pending);
        drop(transients);
        arena
    }
}

fn v4_lookup_input(
    graph: &ActivationGraph,
    input: &ActivationInput,
    singleton: &Arena,
    ancestor_arenas: &[&Arena],
    current_scope: Option<(ScopeFrameId, &Arena)>,
    transients: &TransientStore,
) -> Option<ArenaServiceRef> {
    let dependency = input.dependency?;
    let target = graph.node(dependency);
    match input.storage {
        ActivationInputStorage::Absent => None,
        ActivationInputStorage::Singleton => singleton.lookup(target.identifier),
        ActivationInputStorage::Scoped(owner_frame) => match current_scope {
            Some((current_frame, arena)) if current_frame == owner_frame => {
                arena.lookup(target.identifier)
            }
            _ => ancestor_arenas
                .get(owner_frame.0)
                .and_then(|arena| arena.lookup(target.identifier)),
        },
        ActivationInputStorage::TransientChild
        | ActivationInputStorage::TransientFactoryParameter => {
            transients.lookup(dependency, target.identifier)
        }
    }
}

/// 在同步边界准备一个节点。所有 transient child arena 都在 context 写入稳定指针后才
/// 被从 store 移走，因此 constructor/factory 调用期间地址持续有效。
fn prepare_v4_activation(
    graph: &ActivationGraph,
    id: ActivationNodeId,
    singleton: &Arena,
    ancestor_arenas: &[&Arena],
    current_scope: Option<(ScopeFrameId, &Arena)>,
    transients: &mut TransientStore,
) -> Result<V4PreparedActivation, BuildError> {
    let node = graph.node(id);
    let common = provider_common(&node.provider);
    let mut context = ConstructionContext::new();
    for input in &node.inputs {
        let dependency = v4_lookup_input(
            graph,
            input,
            singleton,
            ancestor_arenas,
            current_scope,
            transients,
        );
        (input.prepare)(&mut context, input.position, dependency)
            .map_err(|error| BuildError::activation(node.identifier, common.source, error))?;
    }

    let children = transients.take_direct_inputs(node);
    let transient_ownership = match node.provider {
        Provider::Class(_) => PreparedTransientOwnership::PersistentChildren(children),
        Provider::Factory(_) => PreparedTransientOwnership::FactoryTemporary(children),
    };
    Ok(V4PreparedActivation {
        id,
        identifier: node.identifier,
        declaration: common.source,
        cleanup: common.cleanup,
        provider: node.provider.clone(),
        context,
        transient_ownership,
    })
}

fn activate_v4_prepared_sync(prepared: V4PreparedActivation) -> V4ActivationOutcome {
    let V4PreparedActivation {
        id,
        identifier,
        declaration,
        cleanup,
        provider,
        context,
        transient_ownership,
    } = prepared;

    match provider {
        Provider::Class(class) => {
            let PreparedTransientOwnership::PersistentChildren(children) = transient_ownership
            else {
                unreachable!("class providers always own field transient children");
            };
            V4ActivationOutcome {
                id,
                identifier,
                declaration,
                cleanup,
                result: (class.constructor)(context),
                children,
            }
        }
        Provider::Factory(factory) => {
            let PreparedTransientOwnership::FactoryTemporary(temporary) = transient_ownership
            else {
                unreachable!("factory providers always own activation-local transient parameters");
            };
            // `temporary` is declared outside the invocation scope.  The factory context and
            // frame are destroyed first; then the parameter-only transient subtree is dropped.
            let result = {
                let frame = FactoryActivationFrame::new();
                match factory.invoker {
                    FactoryInvoker::Sync(constructor) => {
                        constructor(FactoryConstructionContext::from_bound(context, &frame))
                    }
                    FactoryInvoker::Async(_) => unreachable!(
                        "a synchronous activation entry must reject async factories before invocation"
                    ),
                }
            };
            drop(temporary);
            V4ActivationOutcome {
                id,
                identifier,
                declaration,
                cleanup,
                result,
                children: Vec::new(),
            }
        }
    }
}

impl V4PreparedActivation {
    fn into_worker(self, leases: ArenaLeaseSet) -> (V4WorkerJob, PreparedTransientOwnership) {
        let Self {
            id,
            identifier,
            declaration,
            cleanup,
            provider,
            context,
            transient_ownership,
        } = self;
        (
            V4WorkerJob {
                id,
                identifier,
                declaration,
                cleanup,
                provider,
                context,
                leases,
            },
            transient_ownership,
        )
    }
}

/// 在 Tokio worker 内调用一个已准备 provider。
///
/// `FactoryActivationFrame` 从不跨越 worker wrapper 的所有权边界：它与 factory context
/// 和 future 一起在 wrapper 内销毁。class 结果可能含有字段 `Inject`，所以它把 lease
/// 一同交还 coordinator；factory 的 parameter token 不能逃逸，故 invocation 后即可
/// 释放其 lease，再由 coordinator 销毁参数 transient ownership。
async fn activate_v4_worker(job: V4WorkerJob) -> V4WorkerOutcome {
    let V4WorkerJob {
        id,
        identifier,
        declaration,
        cleanup,
        provider,
        context,
        leases,
    } = job;

    match provider {
        Provider::Class(class) => V4WorkerOutcome::Class {
            id,
            identifier,
            declaration,
            cleanup,
            result: (class.constructor)(context),
            // Keep this declaration after `result`: an uncommitted class output containing
            // field tokens must drop before its backing lease disappears.
            leases,
        },
        Provider::Factory(factory) => {
            let result = {
                let frame = FactoryActivationFrame::new();
                match factory.invoker {
                    FactoryInvoker::Sync(constructor) => {
                        constructor(FactoryConstructionContext::from_bound(context, &frame))
                    }
                    FactoryInvoker::Async(constructor) => {
                        constructor(FactoryConstructionContext::from_bound(context, &frame)).await
                    }
                }
            };

            // The frame, context and returned factory future have all been dropped at this
            // point. A `FactoryParameter<'frame>` cannot occur in `result`, so its transient
            // parameter subtree may become eligible for ordinary destruction now.
            drop(leases);
            V4WorkerOutcome::Factory {
                id,
                identifier,
                declaration,
                cleanup,
                result,
            }
        }
    }
}

fn commit_v4_activation(
    graph: &ActivationGraph,
    arena: &mut Arena,
    transients: &mut TransientStore,
    outcome: V4ActivationOutcome,
) -> Result<(), BuildError> {
    let V4ActivationOutcome {
        id,
        identifier,
        declaration,
        cleanup,
        result,
        children,
    } = outcome;
    let service = result.map_err(|error| BuildError::activation(identifier, declaration, error))?;
    commit_v4_service(
        graph,
        arena,
        transients,
        id,
        identifier,
        declaration,
        cleanup,
        service,
        children,
    )
}

// The activation outcome intentionally stays decomposed here: keeping provider diagnostics,
// transient ownership, and the completed service distinct makes the commit/drop ordering auditable.
#[allow(clippy::too_many_arguments)]
fn commit_v4_service(
    graph: &ActivationGraph,
    arena: &mut Arena,
    transients: &mut TransientStore,
    id: ActivationNodeId,
    identifier: ServiceIdentifier,
    declaration: ServiceSource,
    cleanup: Option<crate::registration::provider::CleanupHook>,
    service: ErasedService,
    children: Vec<Arena>,
) -> Result<(), BuildError> {
    let node = graph.node(id);

    if node.lifetime == Lifetime::Transient {
        let mut transient_arena = Arena::new();
        transient_arena
            .commit_with_cleanup_and_children(identifier, service, cleanup, children)
            .map_err(|error| BuildError::arena(identifier, declaration, error))?;
        transients.insert(id, transient_arena);
    } else {
        arena
            .commit_with_cleanup_and_children(identifier, service, cleanup, children)
            .map_err(|error| BuildError::arena(identifier, declaration, error))?;
    }
    Ok(())
}

/// 在 coordinator 中接收 worker outcome，决定 transient ownership 的最终落点，并在
/// class output 完成 commit/ordinary drop 后才释放它保留的 input leases。
fn commit_v4_worker_outcome(
    graph: &ActivationGraph,
    session: &mut V4AsyncActivationSession,
    outcome: V4WorkerOutcome,
) -> Result<(), BuildError> {
    match outcome {
        V4WorkerOutcome::Class {
            id,
            identifier,
            declaration,
            cleanup,
            result,
            leases,
        } => {
            let ownership = session.pending.remove(&id).expect(
                "every spawned class worker must retain its transient ownership in the coordinator",
            );
            let PreparedTransientOwnership::PersistentChildren(children) = ownership else {
                unreachable!("class workers always own field transient children");
            };
            let service = match result {
                Ok(service) => service,
                Err(error) => {
                    drop(children);
                    drop(leases);
                    return Err(BuildError::activation(identifier, declaration, error));
                }
            };
            let committed = commit_v4_service(
                graph,
                &mut session.arena,
                &mut session.transients,
                id,
                identifier,
                declaration,
                cleanup,
                service,
                children,
            );
            drop(leases);
            committed
        }
        V4WorkerOutcome::Factory {
            id,
            identifier,
            declaration,
            cleanup,
            result,
        } => {
            let ownership = session.pending.remove(&id).expect(
                "every spawned factory worker must retain its transient ownership in the coordinator",
            );
            let PreparedTransientOwnership::FactoryTemporary(temporary) = ownership else {
                unreachable!("factory workers always own activation-local transient parameters");
            };
            // The worker has already destroyed its frame/context/future and released the lease.
            // This ordinary drop deliberately does not drive transient cleanup hooks.
            drop(temporary);
            let service =
                result.map_err(|error| BuildError::activation(identifier, declaration, error))?;
            commit_v4_service(
                graph,
                &mut session.arena,
                &mut session.transients,
                id,
                identifier,
                declaration,
                cleanup,
                service,
                Vec::new(),
            )
        }
    }
}

/// 在已经发生其他失败后收口一个 worker outcome。
///
/// 此时 coordinator 不得再提交成功服务或解锁消费者，但仍必须保留每个已完成 worker 的
/// 构造失败，才能按编译图 rank 选择稳定诊断。class output 可能持有 field `Inject`，
/// 因而 value 必须在 lease 之前析构；调用方随后才可普通析构其 pending transient tree。
fn discard_v4_worker_outcome(outcome: V4WorkerOutcome) -> Option<BuildError> {
    match outcome {
        V4WorkerOutcome::Class {
            identifier,
            declaration,
            result,
            leases,
            ..
        } => {
            let failure = match result {
                Ok(service) => {
                    drop(service);
                    None
                }
                Err(error) => Some(BuildError::activation(identifier, declaration, error)),
            };
            drop(leases);
            failure
        }
        V4WorkerOutcome::Factory {
            identifier,
            declaration,
            result,
            ..
        } => result
            .err()
            .map(|error| BuildError::activation(identifier, declaration, error)),
    }
}

fn ensure_v4_sync_schedule(
    graph: &ActivationGraph,
    schedule: &[ActivationNodeId],
) -> Result<(), BuildError> {
    for id in schedule {
        let node = graph.node(*id);
        let common = provider_common(&node.provider);
        if common.cleanup.is_some() {
            return Err(CompileError::UnsupportedCleanup {
                provider: node.identifier,
                declaration: common.source,
            }
            .into());
        }
        if matches!(node.provider, Provider::Factory(ref factory) if matches!(factory.invoker, FactoryInvoker::Async(_)))
        {
            return Err(CompileError::UnsupportedAsyncFactory {
                provider: node.identifier,
                declaration: common.source,
            }
            .into());
        }
    }
    Ok(())
}

fn activate_v4_phase(
    graph: &ActivationGraph,
    schedule: &[ActivationNodeId],
    root: ActivationNodeId,
    outer_singleton: Option<&Arena>,
    ancestor_arenas: &[&Arena],
    current_scope: Option<ScopeFrameId>,
) -> Result<Arena, BuildError> {
    ensure_v4_sync_schedule(graph, schedule)?;
    let mut arena = Arena::new();
    let mut transients = TransientStore::default();
    for id in schedule {
        let singleton = outer_singleton.unwrap_or(&arena);
        let current = current_scope.map(|frame| (frame, &arena));
        let prepared = prepare_v4_activation(
            graph,
            *id,
            singleton,
            ancestor_arenas,
            current,
            &mut transients,
        )?;
        let outcome = activate_v4_prepared_sync(prepared);
        commit_v4_activation(graph, &mut arena, &mut transients, outcome)?;
    }
    debug_assert!(
        transients.is_empty(),
        "every successful transient must be transferred to its consumer"
    );
    debug_assert!(arena.contains(graph.node(root).identifier));
    Ok(arena)
}

fn activate_v4(graph: &ActivationRootGraph) -> Result<Arena, BuildError> {
    activate_v4_phase(
        &graph.graph,
        &graph.graph.order,
        graph.graph.root,
        None,
        &[],
        None,
    )
}

fn activate_v4_scope_singletons(plan: &ActivationScopePlan) -> Result<Arena, BuildError> {
    activate_v4_phase(
        &plan.graph,
        &plan.singleton_order,
        plan.app_root,
        None,
        &[],
        None,
    )
}

fn activate_v4_scope_frame(
    plan: &ActivationScopePlan,
    singleton_arena: &Arena,
    ancestor_arenas: &[&Arena],
    frame: ScopeFrameId,
) -> Result<Arena, BuildError> {
    let scope_level = plan
        .scope_levels
        .get(frame.0)
        .expect("a statically typed Scope must have a matching compiled scope frame");
    activate_v4_phase(
        &plan.graph,
        &scope_level.order,
        scope_level.root,
        Some(singleton_arena),
        ancestor_arenas,
        Some(frame),
    )
}

struct V4ActivationSchedule {
    remaining: BTreeMap<ActivationNodeId, usize>,
    dependents: BTreeMap<ActivationNodeId, Vec<ActivationNodeId>>,
    ready: VecDeque<ActivationNodeId>,
    rank: BTreeMap<ActivationNodeId, usize>,
}

fn v4_activation_schedule(
    graph: &ActivationGraph,
    schedule: &[ActivationNodeId],
) -> V4ActivationSchedule {
    let mut remaining = BTreeMap::new();
    let mut dependents = BTreeMap::new();
    let mut rank = BTreeMap::new();
    for (index, id) in schedule.iter().copied().enumerate() {
        remaining.insert(id, 0);
        dependents.insert(id, Vec::new());
        rank.insert(id, index);
    }
    for id in schedule.iter().copied() {
        let node = graph.node(id);
        for dependency in node.inputs.iter().filter_map(|input| input.dependency) {
            if !remaining.contains_key(&dependency) {
                // A Scoped node reads a pre-built Singleton input from its parent Arena.
                continue;
            }
            *remaining
                .get_mut(&id)
                .expect("each scheduled node has a remaining counter") += 1;
            dependents
                .get_mut(&dependency)
                .expect("each in-phase dependency is scheduled")
                .push(id);
        }
    }
    let ready = schedule
        .iter()
        .copied()
        .filter(|id| remaining.get(id) == Some(&0))
        .collect();
    V4ActivationSchedule {
        remaining,
        dependents,
        ready,
        rank,
    }
}

fn active_tokio_handle() -> Result<Handle, BuildError> {
    Handle::try_current().map_err(|_| BuildError::tokio_runtime_unavailable())
}

/// 为一个 worker 获取它可能透过任何已准备 `Inject` 访问到的完整 Arena 闭包。
///
/// 直接输入之外也纳入同阶段的 parent/ancestor/transient backing：被注入服务的 Rust
/// Drop 可能继续读取自己的依赖，故仅 lease 直接 token 不能覆盖取消后的析构窗口。
fn v4_visible_leases(
    outer_singleton: Option<&Arena>,
    ancestor_arenas: &[&Arena],
    current_arena: &Arena,
    transients: &TransientStore,
) -> ArenaLeaseSet {
    let mut arenas = Vec::with_capacity(
        usize::from(outer_singleton.is_some())
            .saturating_add(ancestor_arenas.len())
            .saturating_add(1)
            .saturating_add(transients.entries.len()),
    );
    if let Some(singleton) = outer_singleton {
        arenas.push(singleton);
    }
    arenas.extend(ancestor_arenas.iter().copied());
    arenas.push(current_arena);
    arenas.extend(transients.arenas());
    ArenaLeaseSet::from_arenas(arenas)
}

// These are the coordinator's independent ownership domains. A context object would obscure
// which inputs are borrowed only for preparation versus mutated scheduling state.
#[allow(clippy::too_many_arguments)]
fn schedule_v4_ready(
    graph: &ActivationGraph,
    outer_singleton: Option<&Arena>,
    ancestor_arenas: &[&Arena],
    current_scope: Option<ScopeFrameId>,
    handle: &Handle,
    ready: &mut VecDeque<ActivationNodeId>,
    task_nodes: &mut BTreeMap<tokio::task::Id, ActivationNodeId>,
    session: &mut V4AsyncActivationSession,
) -> Result<(), (ActivationNodeId, BuildError)> {
    while let Some(id) = ready.pop_front() {
        // Capture the closure before `prepare_v4_activation` transfers direct transient
        // ownership out of the store and into the coordinator's pending table.
        let leases = v4_visible_leases(
            outer_singleton,
            ancestor_arenas,
            &session.arena,
            &session.transients,
        );
        let singleton = outer_singleton.unwrap_or(&session.arena);
        let current = current_scope.map(|frame| (frame, &session.arena));
        let prepared = prepare_v4_activation(
            graph,
            id,
            singleton,
            ancestor_arenas,
            current,
            &mut session.transients,
        )
        .map_err(|error| (id, error))?;
        let (job, ownership) = prepared.into_worker(leases);
        let previous = session.pending.insert(id, ownership);
        debug_assert!(
            previous.is_none(),
            "each activation node can be scheduled only once"
        );
        let abort = session.workers.spawn_on(activate_v4_worker(job), handle);
        let previous = task_nodes.insert(abort.id(), id);
        debug_assert!(
            previous.is_none(),
            "a live Tokio task ID maps to one activation node"
        );
    }
    Ok(())
}

fn v4_worker_join_error(
    graph: &ActivationGraph,
    id: ActivationNodeId,
    error: &tokio::task::JoinError,
) -> BuildError {
    let node = graph.node(id);
    if error.is_panic() {
        BuildError::activation_worker_panicked(
            node.identifier,
            provider_common(&node.provider).source,
        )
    } else {
        BuildError::activation_worker_cancelled(
            node.identifier,
            provider_common(&node.provider).source,
        )
    }
}

/// 以 Tokio `JoinSet` 驱动 occurrence-aware 图。协调器只在 completion 边界访问 Arena；
/// worker 绝不借用它，并以 lease 保护 abort 不等待时仍在析构的构造 frame 与 output。
async fn activate_v4_async_phase(
    graph: &ActivationGraph,
    schedule: &[ActivationNodeId],
    root: ActivationNodeId,
    outer_singleton: Option<&Arena>,
    ancestor_arenas: &[&Arena],
    current_scope: Option<ScopeFrameId>,
) -> Result<Arena, BuildError> {
    let handle = active_tokio_handle()?;
    let V4ActivationSchedule {
        mut remaining,
        dependents,
        mut ready,
        rank,
    } = v4_activation_schedule(graph, schedule);
    let mut session = V4AsyncActivationSession::new();
    let mut task_nodes = BTreeMap::new();
    let mut failures = Vec::new();

    if let Err((id, error)) = schedule_v4_ready(
        graph,
        outer_singleton,
        ancestor_arenas,
        current_scope,
        &handle,
        &mut ready,
        &mut task_nodes,
        &mut session,
    ) {
        failures.push((
            *rank.get(&id).expect("scheduled node has a stable rank"),
            error,
        ));
    }

    while let Some(joined) = session.workers.join_next_with_id().await {
        match joined {
            Ok((task_id, outcome)) => {
                let completed = task_nodes
                    .remove(&task_id)
                    .expect("every JoinSet worker must retain a node mapping");
                debug_assert_eq!(completed, outcome.id());
                let rank_value = *rank
                    .get(&completed)
                    .expect("completed node has a stable rank");

                if failures.is_empty() {
                    match commit_v4_worker_outcome(graph, &mut session, outcome) {
                        Ok(()) => {
                            for dependent in dependents
                                .get(&completed)
                                .expect("completed node has a dependent entry")
                            {
                                let count = remaining
                                    .get_mut(dependent)
                                    .expect("dependent node has a remaining counter");
                                *count = count
                                    .checked_sub(1)
                                    .expect("a dependency cannot complete twice");
                                if *count == 0 {
                                    ready.push_back(*dependent);
                                }
                            }
                            if let Err((id, error)) = schedule_v4_ready(
                                graph,
                                outer_singleton,
                                ancestor_arenas,
                                current_scope,
                                &handle,
                                &mut ready,
                                &mut task_nodes,
                                &mut session,
                            ) {
                                failures.push((
                                    *rank.get(&id).expect("scheduled node has a stable rank"),
                                    error,
                                ));
                            }
                        }
                        Err(error) => failures.push((rank_value, error)),
                    }
                } else {
                    // Stop scheduling after the first observed error but let already-running
                    // workers reach their natural completion. We still retain an activation
                    // error from every completed worker so the final diagnosis is selected by
                    // stable graph rank rather than arrival order. Drop the output before its
                    // coordinator-owned transient tree so class field tokens disappear first.
                    if let Some(error) = discard_v4_worker_outcome(outcome) {
                        failures.push((rank_value, error));
                    }
                    drop(session.pending.remove(&completed));
                }
            }
            Err(error) => {
                let completed = task_nodes
                    .remove(&error.id())
                    .expect("every JoinSet join error must retain a node mapping");
                // Tokio reports this only after the worker future and its frame/context have
                // been dropped. The pending tree can now be ordinarily rolled back.
                drop(session.pending.remove(&completed));
                failures.push((
                    *rank
                        .get(&completed)
                        .expect("cancelled worker has a stable rank"),
                    v4_worker_join_error(graph, completed, &error),
                ));
            }
        }
    }

    debug_assert!(task_nodes.is_empty());
    if let Some((_, error)) = failures.into_iter().min_by_key(|(rank, _)| *rank) {
        return Err(error);
    }
    debug_assert!(session.pending.is_empty());
    debug_assert!(session.transients.is_empty());
    debug_assert!(session.arena.contains(graph.node(root).identifier));
    Ok(session.into_arena())
}

async fn activate_v4_async(graph: &ActivationRootGraph) -> Result<Arena, BuildError> {
    activate_v4_async_phase(
        &graph.graph,
        &graph.graph.order,
        graph.graph.root,
        None,
        &[],
        None,
    )
    .await
}

async fn activate_v4_scope_singletons_async(
    plan: &ActivationScopePlan,
) -> Result<Arena, BuildError> {
    activate_v4_async_phase(
        &plan.graph,
        &plan.singleton_order,
        plan.app_root,
        None,
        &[],
        None,
    )
    .await
}

async fn activate_v4_scope_frame_async(
    plan: &ActivationScopePlan,
    singleton_arena: &Arena,
    ancestor_arenas: &[&Arena],
    frame: ScopeFrameId,
) -> Result<Arena, BuildError> {
    let scope_level = plan
        .scope_levels
        .get(frame.0)
        .expect("a statically typed Scope must have a matching compiled scope frame");
    activate_v4_async_phase(
        &plan.graph,
        &scope_level.order,
        scope_level.root,
        Some(singleton_arena),
        ancestor_arenas,
        Some(frame),
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::{
        marker::PhantomData,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::{
        construction::{
            ErasedService, FactoryConstructionContext, InputPosition, prepare_optional,
            prepare_required,
        },
        inject_wrapper::Inject,
        registration::{
            binding::BoundKeyPolicy,
            provider::{ClassProvider, CleanupFuture, FactoryProvider},
            service_key::ServiceKey,
        },
    };

    struct Database(&'static str);
    struct Controller {
        database: Inject<Database>,
        optional: Option<Inject<OptionalService>>,
    }
    struct OptionalService;
    struct Primary;
    struct Alternative;
    struct Generic<T>(PhantomData<T>);
    struct GenericArgument;
    struct GenericRoot {
        generic: Inject<Generic<GenericArgument>>,
    }
    struct SharedGenericRoot {
        first: Inject<Generic<GenericArgument>>,
        second: Inject<Generic<GenericArgument>>,
    }
    struct MissingRoot;
    struct MissingDependencyRoot;
    struct AmbiguousDependency;
    struct AmbiguousRoot;
    struct CycleA;
    struct CycleB;
    struct AsyncFactoryRoot;
    struct CleanupRoot;
    struct ScopeCompilerAppRoot;
    struct ScopeCompilerRequestRoot;
    struct ScopeCompilerSingleton;
    struct ScopeCompilerLocal;
    struct ScopeCompilerOptional;
    struct ScopeGenericRoot;

    trait Port: Send + Sync {
        fn label(&self) -> &'static str;
    }
    struct Adapter;
    impl Port for Adapter {
        fn label(&self) -> &'static str {
            "adapter"
        }
    }
    struct TraitRoot {
        port: Inject<dyn Port>,
    }

    static MATERIALIZE_CALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);
    static MATERIALIZE_CACHE_CALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);
    static SCOPE_MATERIALIZE_CALLBACK_CALLS: AtomicUsize = AtomicUsize::new(0);

    fn source(line: u32) -> ServiceSource {
        ServiceSource::new("runtime.rs", line, 1)
    }

    fn common(line: u32) -> ProviderCommon {
        ProviderCommon {
            lifetime: Lifetime::Singleton,
            primary: false,
            source: source(line),
            cleanup: None,
        }
    }

    fn scoped_common(line: u32) -> ProviderCommon {
        ProviderCommon {
            lifetime: Lifetime::Scoped,
            ..common(line)
        }
    }

    fn expect_scope_compile_error(
        result: Result<CompiledScopePlan, CompileError>,
        message: &str,
    ) -> CompileError {
        match result {
            Ok(_) => panic!("{message}"),
            Err(error) => error,
        }
    }

    fn identifier<T>() -> ServiceIdentifier
    where
        T: Injectable + ?Sized,
    {
        ServiceIdentifier::from(ServiceType::create::<T>())
    }

    fn construct_database(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(Database("database")))
    }

    fn construct_controller(
        mut context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(Controller {
            database: context.take::<Database>(InputPosition(0))?,
            optional: context.take_optional::<OptionalService>(InputPosition(1))?,
        }))
    }

    fn construct_primary(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(Primary))
    }

    fn construct_alternative(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(Alternative))
    }

    fn construct_generic<T>(_context: ConstructionContext) -> Result<ErasedService, ActivationError>
    where
        T: Injectable,
    {
        Ok(ErasedService::new(Generic::<T>(PhantomData)))
    }

    fn generic_provider<T>() -> Provider
    where
        T: Injectable,
    {
        Provider::Class(ClassProvider {
            provide: identifier::<Generic<T>>(),
            common: common(200),
            dependencies: Vec::new(),
            constructor: construct_generic::<T>,
        })
    }

    fn counted_generic_provider() -> Provider {
        MATERIALIZE_CALLBACK_CALLS.fetch_add(1, Ordering::SeqCst);
        generic_provider::<GenericArgument>()
    }

    fn cached_generic_provider() -> Provider {
        MATERIALIZE_CACHE_CALLBACK_CALLS.fetch_add(1, Ordering::SeqCst);
        generic_provider::<GenericArgument>()
    }

    fn scope_cached_generic_provider() -> Provider {
        SCOPE_MATERIALIZE_CALLBACK_CALLS.fetch_add(1, Ordering::SeqCst);
        generic_provider::<GenericArgument>()
    }

    fn construct_generic_root(
        mut context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(GenericRoot {
            generic: context.take::<Generic<GenericArgument>>(InputPosition(0))?,
        }))
    }

    fn construct_shared_generic_root(
        mut context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(SharedGenericRoot {
            first: context.take::<Generic<GenericArgument>>(InputPosition(0))?,
            second: context.take::<Generic<GenericArgument>>(InputPosition(1))?,
        }))
    }

    fn construct_missing_dependency_root(
        mut context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        let _database = context.take::<Database>(InputPosition(0))?;
        Ok(ErasedService::new(MissingDependencyRoot))
    }

    fn construct_ambiguous_dependency(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(AmbiguousDependency))
    }

    fn construct_ambiguous_root(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(AmbiguousRoot))
    }

    fn construct_cycle_a(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(CycleA))
    }

    fn construct_cycle_b(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(CycleB))
    }

    fn construct_async_factory_root<'frame>(
        _context: FactoryConstructionContext<'frame>,
    ) -> crate::registration::provider::FactoryFuture<'frame> {
        Box::pin(async { Ok(ErasedService::new(AsyncFactoryRoot)) })
    }

    fn construct_cleanup_root(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(CleanupRoot))
    }

    fn construct_scope_compiler_app_root(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(ScopeCompilerAppRoot))
    }

    fn construct_scope_compiler_request_root(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(ScopeCompilerRequestRoot))
    }

    fn construct_scope_compiler_singleton(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(ScopeCompilerSingleton))
    }

    fn construct_scope_compiler_local(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(ScopeCompilerLocal))
    }

    fn construct_scope_generic_root(
        _context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(ScopeGenericRoot))
    }

    fn cleanup_root() -> CleanupFuture {
        Box::pin(async {})
    }

    fn construct_adapter(_context: ConstructionContext) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(Adapter))
    }

    fn construct_trait_root(
        mut context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        Ok(ErasedService::new(TraitRoot {
            port: context.take::<dyn Port>(InputPosition(0))?,
        }))
    }

    fn project_adapter(value: &Adapter) -> &(dyn Port + 'static) {
        value
    }

    fn prepare_adapter_required(
        context: &mut ConstructionContext,
        position: InputPosition,
        input: Option<crate::arena::ArenaServiceRef>,
    ) -> Result<(), ActivationError> {
        crate::construction::prepare_bound_required::<Adapter, dyn Port>(
            context,
            position,
            input,
            project_adapter,
        )
    }

    fn prepare_adapter_optional(
        context: &mut ConstructionContext,
        position: InputPosition,
        input: Option<crate::arena::ArenaServiceRef>,
    ) -> Result<(), ActivationError> {
        crate::construction::prepare_bound_optional::<Adapter, dyn Port>(
            context,
            position,
            input,
            project_adapter,
        )
    }

    #[test]
    fn compiler_activates_only_reachable_singletons_and_optional_absence() {
        let providers = vec![
            Provider::Class(ClassProvider {
                provide: identifier::<Controller>(),
                common: common(10),
                dependencies: vec![
                    DependencyRequest {
                        declaration_position: 0,
                        input_position: InputPosition(0),
                        token: identifier::<Database>(),
                        optional: false,
                        label: Some("database"),
                        delivery: Delivery::Direct(prepare_required::<Database>),
                        provider_source: ProviderSource::Registered,
                    },
                    DependencyRequest {
                        declaration_position: 1,
                        input_position: InputPosition(1),
                        token: identifier::<OptionalService>(),
                        optional: true,
                        label: Some("optional"),
                        delivery: Delivery::Direct(prepare_optional::<OptionalService>),
                        provider_source: ProviderSource::Registered,
                    },
                ],
                constructor: construct_controller,
            }),
            Provider::Class(ClassProvider {
                provide: identifier::<Database>(),
                common: common(11),
                dependencies: Vec::new(),
                constructor: construct_database,
            }),
            Provider::Class(ClassProvider {
                provide: identifier::<Alternative>(),
                common: ProviderCommon {
                    lifetime: Lifetime::Scoped,
                    ..common(12)
                },
                dependencies: Vec::new(),
                constructor: construct_alternative,
            }),
        ];

        let graph = Compiler::new(ProviderRegistry::from_parts(providers, Vec::new()))
            .compile_root::<Controller>()
            .expect("unreachable scoped provider must not affect the root graph");
        let arena = activate(&graph).expect("reachable singleton graph should activate");
        let controller = arena.get::<Controller>().expect("root should be committed");
        assert_eq!(controller.database.0, "database");
        assert!(controller.optional.is_none());
    }

    #[test]
    fn compiler_prefers_a_unique_primary_candidate() {
        let target = identifier::<Primary>();
        let alternate = Provider::Class(ClassProvider {
            provide: target,
            common: common(20),
            dependencies: Vec::new(),
            constructor: construct_primary,
        });
        let primary = Provider::Class(ClassProvider {
            provide: target,
            common: ProviderCommon {
                primary: true,
                ..common(21)
            },
            dependencies: Vec::new(),
            constructor: construct_primary,
        });

        let graph = Compiler::new(ProviderRegistry::from_parts(
            vec![alternate, primary],
            Vec::new(),
        ))
        .compile_root::<Primary>()
        .expect("a unique primary provider should be selected");
        assert_eq!(graph.order, vec![target]);
    }

    #[test]
    fn compiler_materializes_closed_generics_only_when_no_explicit_provider_exists() {
        let generic_identifier = identifier::<Generic<GenericArgument>>();
        let root = Provider::Class(ClassProvider {
            provide: identifier::<GenericRoot>(),
            common: common(30),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: generic_identifier,
                optional: false,
                label: Some("generic"),
                delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                provider_source: ProviderSource::Materialize(generic_provider::<GenericArgument>),
            }],
            constructor: construct_generic_root,
        });

        let graph = Compiler::new(ProviderRegistry::from_parts(vec![root], Vec::new()))
            .compile_root::<GenericRoot>()
            .expect("closed generic provider should materialize");
        assert_eq!(
            graph.order,
            vec![generic_identifier, identifier::<GenericRoot>()]
        );
        let arena = activate(&graph).expect("materialized generic should activate");
        assert!(arena.get::<Generic<GenericArgument>>().is_ok());
        let root = arena
            .get::<GenericRoot>()
            .expect("root should be committed");
        let _ = &root.generic;
    }

    #[test]
    fn compiler_materializes_each_closed_generic_token_once() {
        MATERIALIZE_CACHE_CALLBACK_CALLS.store(0, Ordering::SeqCst);

        let generic_identifier = identifier::<Generic<GenericArgument>>();
        let root = Provider::Class(ClassProvider {
            provide: identifier::<SharedGenericRoot>(),
            common: common(32),
            dependencies: vec![
                DependencyRequest {
                    declaration_position: 0,
                    input_position: InputPosition(0),
                    token: generic_identifier,
                    optional: false,
                    label: Some("first generic"),
                    delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                    provider_source: ProviderSource::Materialize(cached_generic_provider),
                },
                DependencyRequest {
                    declaration_position: 1,
                    input_position: InputPosition(1),
                    token: generic_identifier,
                    optional: false,
                    label: Some("second generic"),
                    delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                    provider_source: ProviderSource::Materialize(cached_generic_provider),
                },
            ],
            constructor: construct_shared_generic_root,
        });

        let graph = Compiler::new(ProviderRegistry::from_parts(vec![root], Vec::new()))
            .compile_root::<SharedGenericRoot>()
            .expect("the repeated closed generic request should compile");

        assert_eq!(
            graph.order,
            vec![generic_identifier, identifier::<SharedGenericRoot>()]
        );
        assert_eq!(
            MATERIALIZE_CACHE_CALLBACK_CALLS.load(Ordering::SeqCst),
            1,
            "the materialize callback must be cached per closed generic token"
        );

        let arena = activate(&graph).expect("the shared generic should activate once");
        let root = arena
            .get::<SharedGenericRoot>()
            .expect("root should be committed");
        let _ = (&root.first, &root.second);
    }

    #[test]
    fn compiler_does_not_materialize_when_an_explicit_closed_generic_provider_exists() {
        MATERIALIZE_CALLBACK_CALLS.store(0, Ordering::SeqCst);

        let generic_identifier = identifier::<Generic<GenericArgument>>();
        let root = Provider::Class(ClassProvider {
            provide: identifier::<GenericRoot>(),
            common: common(35),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: generic_identifier,
                optional: false,
                label: Some("generic"),
                delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                provider_source: ProviderSource::Materialize(counted_generic_provider),
            }],
            constructor: construct_generic_root,
        });
        let explicit = Provider::Class(ClassProvider {
            provide: generic_identifier,
            common: common(36),
            dependencies: Vec::new(),
            constructor: construct_generic::<GenericArgument>,
        });

        let graph = Compiler::new(ProviderRegistry::from_parts(
            vec![root, explicit],
            Vec::new(),
        ))
        .compile_root::<GenericRoot>()
        .expect("an explicit closed generic provider should take precedence");

        assert_eq!(
            graph.order,
            vec![generic_identifier, identifier::<GenericRoot>()]
        );
        assert_eq!(
            MATERIALIZE_CALLBACK_CALLS.load(Ordering::SeqCst),
            0,
            "the fallback callback must not run when an exact explicit provider exists"
        );
    }

    #[test]
    fn compiler_uses_trait_binding_projector() {
        let root = Provider::Class(ClassProvider {
            provide: identifier::<TraitRoot>(),
            common: common(40),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<dyn Port>(),
                optional: false,
                label: Some("port"),
                delivery: Delivery::RequiresBinding,
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_trait_root,
        });
        let adapter = Provider::Class(ClassProvider {
            provide: identifier::<Adapter>(),
            common: common(41),
            dependencies: Vec::new(),
            constructor: construct_adapter,
        });
        let binding = TraitBinding {
            trait_type: ServiceType::create::<dyn Port>(),
            concrete_type: ServiceType::create::<Adapter>(),
            key_policy: BoundKeyPolicy::InheritRequestedKey,
            prepare_required: prepare_adapter_required,
            prepare_optional: prepare_adapter_optional,
            source: source(42),
        };

        let graph = Compiler::new(ProviderRegistry::from_parts(
            vec![root, adapter],
            vec![binding],
        ))
        .compile_root::<TraitRoot>()
        .expect("bound trait should compile");
        let arena = activate(&graph).expect("bound trait should activate");
        assert_eq!(arena.get::<TraitRoot>().unwrap().port.label(), "adapter");
    }

    #[test]
    fn compiler_reports_ambiguous_trait_bindings() {
        let root = Provider::Class(ClassProvider {
            provide: identifier::<TraitRoot>(),
            common: common(45),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<dyn Port>(),
                optional: false,
                label: Some("port"),
                delivery: Delivery::RequiresBinding,
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_trait_root,
        });
        let later_binding = TraitBinding {
            trait_type: ServiceType::create::<dyn Port>(),
            concrete_type: ServiceType::create::<Adapter>(),
            key_policy: BoundKeyPolicy::InheritRequestedKey,
            prepare_required: prepare_adapter_required,
            prepare_optional: prepare_adapter_optional,
            source: source(47),
        };
        let earlier_binding = TraitBinding {
            source: source(46),
            ..later_binding
        };

        let error = match Compiler::new(ProviderRegistry::from_parts(
            vec![root],
            vec![later_binding, earlier_binding],
        ))
        .compile_root::<TraitRoot>()
        {
            Ok(_) => panic!("multiple bindings for one trait must be rejected"),
            Err(error) => error,
        };

        match error {
            CompileError::AmbiguousTraitBinding {
                provider,
                dependency,
                candidates,
            } => {
                assert_eq!(provider, identifier::<TraitRoot>());
                assert_eq!(dependency, identifier::<dyn Port>());
                assert_eq!(candidates, vec![source(46), source(47)]);
            }
            other => panic!("expected an ambiguous trait binding error, got {other:?}"),
        }
    }

    #[test]
    fn compiler_reports_unsupported_reachable_lifetime() {
        let provider = Provider::Class(ClassProvider {
            provide: identifier::<Alternative>(),
            common: ProviderCommon {
                lifetime: Lifetime::Transient,
                ..common(50)
            },
            dependencies: Vec::new(),
            constructor: construct_alternative,
        });

        let error = match Compiler::new(ProviderRegistry::from_parts(vec![provider], Vec::new()))
            .compile_root::<Alternative>()
        {
            Ok(_) => panic!("reachable transient provider must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(error, CompileError::UnsupportedLifetime { .. }));
    }

    #[test]
    fn compiler_reports_missing_root() {
        let error = match Compiler::new(ProviderRegistry::from_parts(Vec::new(), Vec::new()))
            .compile_root::<MissingRoot>()
        {
            Ok(_) => panic!("an unregistered root must be rejected"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            CompileError::MissingRoot { root } if root == identifier::<MissingRoot>()
        ));
    }

    #[test]
    fn compiler_reports_missing_required_dependency() {
        let provider = Provider::Class(ClassProvider {
            provide: identifier::<MissingDependencyRoot>(),
            common: common(76),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<Database>(),
                optional: false,
                label: Some("database"),
                delivery: Delivery::Direct(prepare_required::<Database>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_missing_dependency_root,
        });

        let error = match Compiler::new(ProviderRegistry::from_parts(vec![provider], Vec::new()))
            .compile_root::<MissingDependencyRoot>()
        {
            Ok(_) => panic!("a required dependency without a provider must fail compilation"),
            Err(error) => error,
        };

        match error {
            CompileError::MissingDependency {
                provider,
                dependency,
                label,
            } => {
                assert_eq!(provider, identifier::<MissingDependencyRoot>());
                assert_eq!(dependency, identifier::<Database>());
                assert_eq!(label, Some("database"));
            }
            other => panic!("expected a missing dependency error, got {other:?}"),
        }
    }

    #[test]
    fn compiler_rejects_ambiguous_primary_candidates_and_cycles() {
        let root = Provider::Class(ClassProvider {
            provide: identifier::<AmbiguousRoot>(),
            common: common(80),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<AmbiguousDependency>(),
                optional: false,
                label: Some("ambiguous dependency"),
                delivery: Delivery::Direct(prepare_required::<AmbiguousDependency>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_ambiguous_root,
        });
        let first_primary = Provider::Class(ClassProvider {
            provide: identifier::<AmbiguousDependency>(),
            common: ProviderCommon {
                primary: true,
                ..common(81)
            },
            dependencies: Vec::new(),
            constructor: construct_ambiguous_dependency,
        });
        let second_primary = Provider::Class(ClassProvider {
            provide: identifier::<AmbiguousDependency>(),
            common: ProviderCommon {
                primary: true,
                ..common(82)
            },
            dependencies: Vec::new(),
            constructor: construct_ambiguous_dependency,
        });

        let error = match Compiler::new(ProviderRegistry::from_parts(
            vec![root, first_primary, second_primary],
            Vec::new(),
        ))
        .compile_root::<AmbiguousRoot>()
        {
            Ok(_) => panic!("multiple primary candidates must remain ambiguous"),
            Err(error) => error,
        };

        match error {
            CompileError::AmbiguousDependency {
                provider,
                dependency,
                candidates,
            } => {
                assert_eq!(provider, identifier::<AmbiguousRoot>());
                assert_eq!(dependency, identifier::<AmbiguousDependency>());
                assert_eq!(candidates, vec![source(81), source(82)]);
            }
            other => panic!("expected an ambiguous dependency error, got {other:?}"),
        }

        let cycle_a = Provider::Class(ClassProvider {
            provide: identifier::<CycleA>(),
            common: common(90),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<CycleB>(),
                optional: false,
                label: Some("cycle b"),
                delivery: Delivery::Direct(prepare_required::<CycleB>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_cycle_a,
        });
        let cycle_b = Provider::Class(ClassProvider {
            provide: identifier::<CycleB>(),
            common: common(91),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<CycleA>(),
                optional: false,
                label: Some("cycle a"),
                delivery: Delivery::Direct(prepare_required::<CycleA>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_cycle_b,
        });

        let error = match Compiler::new(ProviderRegistry::from_parts(
            vec![cycle_a, cycle_b],
            Vec::new(),
        ))
        .compile_root::<CycleA>()
        {
            Ok(_) => panic!("a reachable provider cycle must be rejected"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            CompileError::Cycle { chain }
                if chain == vec![identifier::<CycleA>(), identifier::<CycleB>(), identifier::<CycleA>()]
        ));
    }

    #[test]
    fn compiler_rejects_async_factory() {
        let provider = Provider::Factory(FactoryProvider {
            provide: identifier::<AsyncFactoryRoot>(),
            common: common(100),
            dependencies: Vec::new(),
            invoker: FactoryInvoker::Async(construct_async_factory_root),
        });

        let error = match Compiler::new(ProviderRegistry::from_parts(vec![provider], Vec::new()))
            .compile_root::<AsyncFactoryRoot>()
        {
            Ok(_) => panic!("v0 must reject reachable async factories"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            CompileError::UnsupportedAsyncFactory { provider, .. }
                if provider == identifier::<AsyncFactoryRoot>()
        ));
    }

    #[test]
    fn compiler_rejects_cleanup_provider() {
        let provider = Provider::Class(ClassProvider {
            provide: identifier::<CleanupRoot>(),
            common: ProviderCommon {
                cleanup: Some(cleanup_root),
                ..common(110)
            },
            dependencies: Vec::new(),
            constructor: construct_cleanup_root,
        });

        let error = match Compiler::new(ProviderRegistry::from_parts(vec![provider], Vec::new()))
            .compile_root::<CleanupRoot>()
        {
            Ok(_) => panic!("the synchronous compiler must reject reachable cleanup hooks"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            CompileError::UnsupportedCleanup { provider, .. }
                if provider == identifier::<CleanupRoot>()
        ));
    }

    #[test]
    fn async_compiler_accepts_reachable_cleanup_provider() {
        let provider = Provider::Class(ClassProvider {
            provide: identifier::<CleanupRoot>(),
            common: ProviderCommon {
                cleanup: Some(cleanup_root),
                ..common(111)
            },
            dependencies: Vec::new(),
            constructor: construct_cleanup_root,
        });

        let graph = Compiler::new(ProviderRegistry::from_parts(vec![provider], Vec::new()))
            .compile_async_root::<CleanupRoot>()
            .expect("the async compiler should preserve a reachable cleanup hook");

        assert_eq!(graph.order, vec![identifier::<CleanupRoot>()]);
        assert!(
            provider_common(
                &graph
                    .providers
                    .get(&identifier::<CleanupRoot>())
                    .expect("the cleanup root should remain in the compiled graph")
                    .provider,
            )
            .cleanup
            .is_some(),
            "cleanup metadata must remain attached to the compiled provider"
        );
    }

    #[test]
    fn keys_remain_part_of_the_provider_identity() {
        let keyed = ServiceIdentifier::new(
            Some(ServiceKey::Named("secondary")),
            ServiceType::create::<Database>(),
        );
        let root = Provider::Class(ClassProvider {
            provide: identifier::<Controller>(),
            common: common(60),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: keyed,
                optional: false,
                label: Some("keyed database"),
                delivery: Delivery::Direct(prepare_required::<Database>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_controller,
        });
        let keyed_database = Provider::Class(ClassProvider {
            provide: keyed,
            common: common(61),
            dependencies: Vec::new(),
            constructor: construct_database,
        });

        let graph = Compiler::new(ProviderRegistry::from_parts(
            vec![root, keyed_database],
            Vec::new(),
        ))
        .compile_root::<Controller>()
        .expect("a keyed dependency should resolve only its exact token");
        assert_eq!(graph.order, vec![keyed, identifier::<Controller>()]);
    }

    #[test]
    fn compiler_scope_plan_marks_each_input_with_its_explicit_arena_source() {
        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(120),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let singleton = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerSingleton>(),
            common: common(121),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_singleton,
        });
        let local = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerLocal>(),
            common: scoped_common(122),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_local,
        });
        let scope_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: scoped_common(123),
            dependencies: vec![
                DependencyRequest {
                    declaration_position: 0,
                    input_position: InputPosition(0),
                    token: identifier::<ScopeCompilerSingleton>(),
                    optional: false,
                    label: Some("singleton"),
                    delivery: Delivery::Direct(prepare_required::<ScopeCompilerSingleton>),
                    provider_source: ProviderSource::Registered,
                },
                DependencyRequest {
                    declaration_position: 1,
                    input_position: InputPosition(1),
                    token: identifier::<ScopeCompilerLocal>(),
                    optional: false,
                    label: Some("local"),
                    delivery: Delivery::Direct(prepare_required::<ScopeCompilerLocal>),
                    provider_source: ProviderSource::Registered,
                },
                DependencyRequest {
                    declaration_position: 2,
                    input_position: InputPosition(2),
                    token: identifier::<ScopeCompilerOptional>(),
                    optional: true,
                    label: Some("optional"),
                    delivery: Delivery::Direct(prepare_optional::<ScopeCompilerOptional>),
                    provider_source: ProviderSource::Registered,
                },
            ],
            constructor: construct_scope_compiler_request_root,
        });

        let plan = Compiler::new(ProviderRegistry::from_parts(
            vec![app_root, singleton, local, scope_root],
            Vec::new(),
        ))
        .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>()
        .expect("the dual-root graph should compile");

        assert_eq!(
            plan.singleton_order,
            vec![
                identifier::<ScopeCompilerAppRoot>(),
                identifier::<ScopeCompilerSingleton>()
            ]
        );
        assert_eq!(
            plan.scoped_order,
            vec![
                identifier::<ScopeCompilerLocal>(),
                identifier::<ScopeCompilerRequestRoot>()
            ]
        );

        let inputs = &plan
            .providers
            .get(&identifier::<ScopeCompilerRequestRoot>())
            .expect("scope root must be part of the compiled plan")
            .inputs;
        assert_eq!(inputs[0].storage, InputStorage::Singleton);
        assert_eq!(inputs[1].storage, InputStorage::Scoped);
        assert_eq!(inputs[2].storage, InputStorage::Absent);

        let singleton_arena =
            activate_scope_singletons(&plan).expect("singleton segment should activate once");
        let scoped_arena =
            activate_scoped(&plan, &singleton_arena).expect("scoped segment should activate");
        assert!(singleton_arena.contains(identifier::<ScopeCompilerAppRoot>()));
        assert!(singleton_arena.contains(identifier::<ScopeCompilerSingleton>()));
        assert!(scoped_arena.contains(identifier::<ScopeCompilerLocal>()));
        assert!(scoped_arena.contains(identifier::<ScopeCompilerRequestRoot>()));
    }

    #[test]
    fn compiler_scope_plan_reports_structured_root_lifetime_errors() {
        let invalid_app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: scoped_common(130),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: scoped_common(131),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_request_root,
        });

        let error = expect_scope_compile_error(
            Compiler::new(ProviderRegistry::from_parts(
                vec![invalid_app_root, request_root],
                Vec::new(),
            ))
            .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>(),
            "the app root must be a Singleton",
        );
        assert!(matches!(
            error,
            CompileError::InvalidRootLifetime {
                root,
                expected: Lifetime::Singleton,
                actual: Lifetime::Scoped,
                ..
            } if root == identifier::<ScopeCompilerAppRoot>()
        ));

        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(132),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let invalid_request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: common(133),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_request_root,
        });

        let error = expect_scope_compile_error(
            Compiler::new(ProviderRegistry::from_parts(
                vec![app_root, invalid_request_root],
                Vec::new(),
            ))
            .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>(),
            "the request root must be Scoped",
        );
        assert!(matches!(
            error,
            CompileError::InvalidRootLifetime {
                root,
                expected: Lifetime::Scoped,
                actual: Lifetime::Singleton,
                ..
            } if root == identifier::<ScopeCompilerRequestRoot>()
        ));
    }

    #[test]
    fn compiler_scope_plan_rejects_singleton_to_scoped_lifetime_inversion() {
        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(140),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<ScopeCompilerLocal>(),
                optional: false,
                label: Some("illegal scoped dependency"),
                delivery: Delivery::Direct(prepare_required::<ScopeCompilerLocal>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_scope_compiler_app_root,
        });
        let local = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerLocal>(),
            common: scoped_common(141),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_local,
        });
        let request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: scoped_common(142),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_request_root,
        });

        let error = expect_scope_compile_error(
            Compiler::new(ProviderRegistry::from_parts(
                vec![app_root, local, request_root],
                Vec::new(),
            ))
            .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>(),
            "a Singleton must not capture a Scoped service",
        );

        assert!(matches!(
            error,
            CompileError::LifetimeInversion {
                provider,
                provider_lifetime: Lifetime::Singleton,
                dependency,
                dependency_lifetime: Lifetime::Scoped,
                ..
            } if provider == identifier::<ScopeCompilerAppRoot>()
                && dependency == identifier::<ScopeCompilerLocal>()
        ));
    }

    #[test]
    fn compiler_scope_plan_caches_closed_generic_materialization_across_both_roots() {
        SCOPE_MATERIALIZE_CALLBACK_CALLS.store(0, Ordering::SeqCst);

        let generic = identifier::<Generic<GenericArgument>>();
        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<GenericRoot>(),
            common: common(150),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: generic,
                optional: false,
                label: Some("app generic"),
                delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                provider_source: ProviderSource::Materialize(scope_cached_generic_provider),
            }],
            constructor: construct_generic_root,
        });
        let request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeGenericRoot>(),
            common: scoped_common(151),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: generic,
                optional: false,
                label: Some("scope generic"),
                delivery: Delivery::Direct(prepare_required::<Generic<GenericArgument>>),
                provider_source: ProviderSource::Materialize(scope_cached_generic_provider),
            }],
            constructor: construct_scope_generic_root,
        });

        let plan = Compiler::new(ProviderRegistry::from_parts(
            vec![app_root, request_root],
            Vec::new(),
        ))
        .compile_scope_plan::<GenericRoot, ScopeGenericRoot>()
        .expect("the same closed generic should be shared by both roots");

        assert_eq!(
            SCOPE_MATERIALIZE_CALLBACK_CALLS.load(Ordering::SeqCst),
            1,
            "one compiler session must materialize a closed generic at most once"
        );
        assert_eq!(
            plan.singleton_order
                .iter()
                .filter(|identifier| **identifier == generic)
                .count(),
            1
        );
        assert_eq!(
            plan.providers
                .get(&identifier::<ScopeGenericRoot>())
                .expect("scope root must remain in the plan")
                .inputs[0]
                .storage,
            InputStorage::Singleton
        );
    }

    #[test]
    fn compiler_scope_plan_keeps_transient_and_sync_cleanup_boundaries() {
        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(160),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: scoped_common(161),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_position: InputPosition(0),
                token: identifier::<Alternative>(),
                optional: false,
                label: Some("transient"),
                delivery: Delivery::Direct(prepare_required::<Alternative>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_scope_compiler_request_root,
        });
        let transient = Provider::Class(ClassProvider {
            provide: identifier::<Alternative>(),
            common: ProviderCommon {
                lifetime: Lifetime::Transient,
                ..common(162)
            },
            dependencies: Vec::new(),
            constructor: construct_alternative,
        });

        let error = expect_scope_compile_error(
            Compiler::new(ProviderRegistry::from_parts(
                vec![app_root, request_root, transient],
                Vec::new(),
            ))
            .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>(),
            "reachable Transient providers remain unsupported in a scope plan",
        );
        assert!(matches!(
            error,
            CompileError::UnsupportedLifetime {
                provider,
                lifetime: Lifetime::Transient,
                ..
            } if provider == identifier::<Alternative>()
        ));

        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(163),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let cleanup_request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: ProviderCommon {
                cleanup: Some(cleanup_root),
                ..scoped_common(164)
            },
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_request_root,
        });

        let error = expect_scope_compile_error(
            Compiler::new(ProviderRegistry::from_parts(
                vec![app_root, cleanup_request_root],
                Vec::new(),
            ))
            .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>(),
            "reachable cleanup hooks remain unsupported in a scope plan",
        );
        assert!(matches!(
            error,
            CompileError::UnsupportedCleanup { provider, .. }
                if provider == identifier::<ScopeCompilerRequestRoot>()
        ));

        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(165),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let cleanup_request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: ProviderCommon {
                cleanup: Some(cleanup_root),
                ..scoped_common(166)
            },
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_request_root,
        });

        let plan = Compiler::new(ProviderRegistry::from_parts(
            vec![app_root, cleanup_request_root],
            Vec::new(),
        ))
        .compile_async_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>()
        .expect("the async scope compiler should preserve a reachable cleanup hook");
        assert_eq!(
            plan.scoped_order,
            vec![identifier::<ScopeCompilerRequestRoot>()]
        );
        assert!(
            provider_common(
                &plan
                    .providers
                    .get(&identifier::<ScopeCompilerRequestRoot>())
                    .expect("the cleanup request root should remain in the async scope plan")
                    .provider,
            )
            .cleanup
            .is_some()
        );

        let app_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerAppRoot>(),
            common: common(167),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_app_root,
        });
        let request_root = Provider::Class(ClassProvider {
            provide: identifier::<ScopeCompilerRequestRoot>(),
            common: scoped_common(168),
            dependencies: Vec::new(),
            constructor: construct_scope_compiler_request_root,
        });
        let unrelated_transient = Provider::Class(ClassProvider {
            provide: identifier::<Alternative>(),
            common: ProviderCommon {
                lifetime: Lifetime::Transient,
                ..common(169)
            },
            dependencies: Vec::new(),
            constructor: construct_alternative,
        });

        Compiler::new(ProviderRegistry::from_parts(
            vec![app_root, request_root, unrelated_transient],
            Vec::new(),
        ))
        .compile_scope_plan::<ScopeCompilerAppRoot, ScopeCompilerRequestRoot>()
        .expect("unrelated unsupported registrations must not affect the two-root plan");
    }
}
