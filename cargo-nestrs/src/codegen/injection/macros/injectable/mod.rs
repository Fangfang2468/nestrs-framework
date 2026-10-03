//! Class 服务的字段分析、声明改写、构造适配与 provider 描述生成。
//!
//! 各阶段共享同一份字段事实；开放泛型声明保持蓝图，闭合服务才生成最终执行适配器。

pub mod config;
mod constructor;
pub mod field_analyze;
mod field_initialization;
pub mod injection_field_rewrite;
mod provider;
mod provider_definition;
mod registration;

pub(crate) use constructor::GenerateInjectableConstructor;
pub(crate) use field_analyze::analyze_fields;
pub(crate) use injection_field_rewrite::rewrite_injection_field;
pub(crate) use provider::CollectInjectableProvider;
pub(crate) use provider_definition::DefineGenericInjectableProvider;
pub(crate) use registration::EmitInjectableRegistration;
