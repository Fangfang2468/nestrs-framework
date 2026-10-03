//! 服务声明级配置及相邻属性的展开顺序协调。
//!
//! 这里只解析策略事实；字段和参数上的同名 helper 使用 sub_macros 中的独立规则。

pub mod cleanup;
pub(crate) mod lazy;
pub mod lifetime;
pub(crate) mod primary;
pub mod service_key;
