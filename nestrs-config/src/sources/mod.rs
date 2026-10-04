//! 不可变配置构建器的内置来源。
//!
//! [`Memory`] 和 [`Environment`] 始终可用；文件来源分别由 `json`、`toml`、`dotenv`
//! feature 控制。创建来源只保存读取选项，真正加载发生在构建器调用
//! [`crate::ConfigSource::load`] 时；来源只返回自己的配置层，不决定层间覆盖顺序。
//! 文件及环境值不会通过来源的 `Debug` 输出，原始 I/O、解析器错误也不会进入公开错误链。

mod environment;
mod memory;

#[cfg(feature = "dotenv")]
mod dotenv;
#[cfg(any(feature = "json", feature = "toml", feature = "dotenv"))]
mod file;
#[cfg(feature = "json")]
mod json;
#[cfg(feature = "toml")]
mod toml;

#[cfg(feature = "dotenv")]
pub use dotenv::DotEnv;
pub use environment::Environment;
#[cfg(feature = "json")]
pub use json::Json;
pub use memory::Memory;
#[cfg(feature = "toml")]
pub use toml::Toml;
