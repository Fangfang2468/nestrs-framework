//! bind 宏生成的 concrete-to-trait 投影元数据 ABI。

use linkme::distributed_slice;

use crate::{
    activation::InputPreparer,
    registration::dependency::ClosedProviderCallback,
    service::{ServiceSource, ServiceType},
};

/// 将一个 concrete provider 的稳定地址投影为 trait-object 输入的规则。
#[derive(Debug, Clone, Copy)]
pub struct TraitBinding {
    pub trait_type: ServiceType,
    pub concrete_type: ServiceType,
    /// 已知闭合 self 类型的可选蓝图；factory-only 类型不要求 ProviderDefinition。
    /// 图编译器在绑定校验前展开它，避免仅查询 dyn Trait 时遗漏其泛型实现。
    pub materialize: Option<ClosedProviderCallback>,
    pub key_policy: BoundKeyPolicy,
    pub prepare_required: InputPreparer,
    pub prepare_optional: InputPreparer,
    pub source: ServiceSource,
}

/// trait 请求携带的 key 如何传递给 concrete provider。
///
/// bind 宏只描述这个策略；图编译器按策略冻结服务身份与投影。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundKeyPolicy {
    InheritRequestedKey,
}

/// 当前链接单元内由 bind 宏声明的 trait binding。
#[distributed_slice]
pub static REFLECTED_BINDINGS: [fn() -> TraitBinding] = [..];
