pub mod config;
mod component_definition;
mod constructor;
pub mod field_analyze;
mod field_initialization;
pub mod injection_field_rewrite;
mod metadata;
mod registration;

pub(crate) use component_definition::DefineGenericInjectableComponent;
pub(crate) use constructor::GenerateInjectableConstructor;
pub(crate) use field_analyze::analyze_fields;
pub(crate) use injection_field_rewrite::RewriteInjectionField;
pub(crate) use metadata::CollectInjectableMetadata;
pub(crate) use registration::EmitInjectableRegistration;
