//! 高级静态 Scope API。
//!
//! 普通应用只需使用根路径的 [`crate::ServiceProvider`]。只有需要预声明 Scoped 生命周期
//! 链时，才应从本模块导入 `ScopeProvider`、`ScopeLayer` 与 `ScopeEnd`。

pub use crate::runtime::{Scope, ScopeEnd, ScopeLayer, ScopeProvider};

/// 仅供 `ScopeLayer` 类型链在公开泛型约束中使用的密封实现细节。
///
/// 应用代码只组合 `ScopeLayer` 和 `ScopeEnd`，不应实现或直接调用这些 trait。
#[doc(hidden)]
pub mod __private {
    pub use crate::runtime::{ScopeChain, ScopeTail};
}
