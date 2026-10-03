//! 容器启动选项。类型独立于门面，供编译计划装配和运行时共同使用。
//!
//! Default 表达库的基线，不读取文件或执行注册回调。项目 Cargo.toml 的选项由工具链
//! 固化到当前入口的执行计划；build 使用计划默认值，build_with_options 完整显式覆盖。

use std::num::NonZeroUsize;

/// root 的初始化默认值；单个服务上的 #[lazy] / #[lazy(false)] 优先于此默认值。
/// 主动预热只选择 Singleton 入口及其必要依赖，不改变 Scoped/Transient 的生命周期。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InitializationMode {
    /// 默认等待查询或依赖需求；显式 #[lazy(false)] Singleton 仍在 build 时预热。
    #[default]
    Lazy,
    /// 构建时默认预热 Singleton；#[lazy] 排除独立入口，但不阻止普通依赖需要它。
    Eager,
}

/// 所有 scope 共享 root 的构造并发上限。
#[derive(Debug, Clone)]
pub struct ServiceProviderOptions {
    /// 入口的全局初始化默认值；服务声明级覆盖优先，选中的预热在 build 返回前完成。
    pub initialization: InitializationMode,
    /// 整个 root 及所有 scope 共享的构造任务上限，不限制业务服务方法的执行并发。
    pub max_concurrent_activations: NonZeroUsize,
}

impl Default for ServiceProviderOptions {
    fn default() -> Self {
        Self {
            initialization: InitializationMode::Lazy,
            max_concurrent_activations: NonZeroUsize::new(32).unwrap(),
        }
    }
}
