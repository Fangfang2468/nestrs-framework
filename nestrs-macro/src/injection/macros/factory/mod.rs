//! `#[factory]` 的 Provider 分析与生成。
//!
//! 入口宏应先完成模块作用域、private、unsafe/extern 与 primary marker 的协调，再：
//!
//! 1. 通过 [`parse_factory_config`] 解析 provider 属性；
//! 2. 通过 [`analyze_factory`] 消费参数 marker、拒绝不支持签名并获得重写函数；
//! 3. 用 [`RewriteFactorySignature`] 输出用户函数；
//! 4. 用 [`EmitFactoryProvider`] 输出同一 factory 的匿名 adapter/linkme 注册。
//!
//! `primary` 不属于 factory 参数配置；入口在处理 `#[primary]` 的源码顺序后，把最终
//! 布尔值传给 `EmitFactoryProvider`，从而任意属性顺序都只生成一项 Factory provider。

mod analyze;
mod codegen;
mod config;

pub(crate) use analyze::{
    FactoryAnalysis, FactoryInvocation, FactoryParameterSpec, FactoryResultKind, analyze_factory,
};
pub(crate) use codegen::{EmitFactoryProvider, RewriteFactorySignature};
pub(crate) use config::{FactoryConfig, parse_factory_config};
