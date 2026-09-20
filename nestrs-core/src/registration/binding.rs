//! bind 宏生成的 concrete-to-trait 投影元数据 ABI。

use linkme::distributed_slice;

use crate::{
    construction::PrepareInput,
    registration::{service_source::ServiceSource, service_type::ServiceType},
};

/// 将一个 concrete provider 的稳定地址投影为 trait-object 输入的规则。
#[derive(Debug, Clone, Copy)]
pub struct TraitBinding {
    pub trait_type: ServiceType,
    pub concrete_type: ServiceType,
    pub key_policy: BoundKeyPolicy,
    pub prepare_required: PrepareInput,
    pub prepare_optional: PrepareInput,
    pub source: ServiceSource,
}

/// trait 请求携带的 key 如何传递给 concrete provider。
///
/// bind 宏只描述这个策略；后续容器负责按策略解析服务身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundKeyPolicy {
    InheritRequestedKey,
}

/// 当前链接单元内由 bind 宏声明的 trait binding。
#[distributed_slice]
pub static REFLECTED_BINDINGS: [fn() -> TraitBinding] = [..];
