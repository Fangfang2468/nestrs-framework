//! Nestrs 构建工具与声明后端。工具私有的声明桥接在编译期复用此库的生成入口；
//! 应用运行时只链接 `nestrs-core`，不链接 CLI 或编译器适配器。

#[path = "protocol.rs"]
mod protocol;

pub mod bridge;

#[doc(hidden)]
pub mod codegen;

pub mod commands;

#[doc(hidden)]
pub mod di_plan;

pub mod graph;

#[doc(hidden)]
pub mod ide;

#[doc(hidden)]
pub mod project_config;

pub mod toolchain;
