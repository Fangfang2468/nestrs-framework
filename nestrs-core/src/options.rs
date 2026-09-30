//! 容器启动选项。类型独立于门面，供门面和运行时共同使用。
//!
//! Default 表达库的基线，不读取文件或执行注册回调。

use std::num::NonZeroUsize;

/// 提前初始化的范围。Eager 只预热 Singleton 及其必要依赖。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum InitializationMode {
    /// 仅验证并冻结图，首次查询时才构造对应依赖闭包。
    #[default]
    Lazy,
    /// 构建容器时完成全部 Singleton 及其必要依赖的初始化。
    Eager,
}

/// 所有 scope 共享 root 的构造并发上限。
#[derive(Debug, Clone)]
pub struct ServiceProviderOptions {
    /// Lazy 按查询激活；Eager 在 build 返回前预热 Singleton 及其必要依赖。
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
