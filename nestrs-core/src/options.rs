//! 容器启动选项。类型独立于门面，供编译计划装配和运行时共同使用。
//!
//! Default 表达库的基线，不读取文件或执行注册回调。项目 Cargo.toml 的选项由工具链
//! 固化到当前入口的执行计划；build(None) 使用计划默认值，build(Some(options)) 完整覆盖。

use std::num::NonZeroUsize;

/// owner 创建时的初始化默认值；服务上的 `#[lazy]` / `#[lazy(false)]` 优先。
/// root 选择 Singleton，scope 选择 Scoped；均不改变服务的生命周期。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InitializationMode {
    /// 默认等待查询或依赖需求；显式 `#[lazy(false)]` 服务仍在所属 owner 创建时初始化。
    #[default]
    Lazy,

    /// 创建时完成所属生命周期的服务初始化；`#[lazy]` 排除独立入口，普通依赖仍可需要它。
    Eager,
}

/// root / scope 的独立初始化默认值及共享构造并发上限。
#[derive(Debug, Clone)]
pub struct ServiceProviderOptions {
    /// root 的初始化默认值；服务声明级覆盖优先，选中的 Singleton 在 build 返回前完成。
    pub initialization: InitializationMode,

    /// 每次创建 scope 时采用的独立默认策略，不继承 root 的 initialization。
    /// 单次 create_scope(Some(options)) 可以覆盖此值。
    pub scope_initialization: InitializationMode,

    /// 整个 root 及所有 scope 共享的活跃构造 worker 上限，默认 32。
    /// 等待依赖的任务不占名额；此值不限制请求队列、实例总数、业务方法或 cleanup 并发。
    pub max_concurrent_activations: NonZeroUsize,
}

impl Default for ServiceProviderOptions {
    /// 返回 root Lazy / scope Lazy / 32 基线，不读取入口项目的编译配置。
    fn default() -> Self {
        Self {
            initialization: InitializationMode::Lazy,
            scope_initialization: InitializationMode::Lazy,
            max_concurrent_activations: NonZeroUsize::new(32).unwrap(),
        }
    }
}

/// create_scope(Some(options)) 的单次创建选项，完整覆盖容器保存的 scope 默认值。
///
/// 构造并发额度仍由 root 管理，不为 scope 建立独立调度器或额度。
#[derive(Debug, Clone, Copy, Default)]
pub struct ServiceScopeOptions {
    /// 返回 scope 之前需要完成的初始化策略，库默认值为 Lazy。
    pub initialization: InitializationMode,
}
