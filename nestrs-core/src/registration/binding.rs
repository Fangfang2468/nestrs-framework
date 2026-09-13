//! trait 绑定（`#[bind]`）的注册 ABI。
//!
//! 绑定不是 provider：它不构造实例、不导出自己的 service token，也没有 lifetime、
//! primary 或 cleanup。它只描述「请求 `dyn Trait` 时应当激活哪个 concrete 服务，以及
//! 如何把已提交的 concrete 地址投影成 trait object」，因此与实例 provider 分开收集。
//!
//! resolver 的动作顺序是：按 [`BoundKeyPolicy`] 从 trait 请求派生 concrete token →
//! 激活对应 provider → 用这里的 projector 把稳定地址写入消费方槽位。

use linkme::distributed_slice;

use crate::{
    construction::PrepareInput,
    registration::{
        service_identifier::ServiceIdentifier, service_source::ServiceSource,
        service_type::ServiceType,
    },
};

/// 将一个 concrete provider 的已提交地址投影为 trait-object 注入输入的规则。
#[derive(Debug, Clone, Copy)]
pub struct TraitBinding {
    /// 被导出的 trait 类型。实际请求 key 由 `key_policy` 解释。
    pub trait_type: ServiceType,

    /// 实际需要激活的 concrete 服务类型。
    pub concrete_type: ServiceType,

    /// concrete token 如何继承 trait 请求的 key。
    pub key_policy: BoundKeyPolicy,

    /// concrete-to-trait 必选投影函数。
    pub prepare_required: PrepareInput,

    /// concrete-to-trait 可选投影函数。
    pub prepare_optional: PrepareInput,

    /// bind 声明来源。
    pub source: ServiceSource,
}

/// `#[bind]` 将 trait 请求映射到 concrete provider 时采用的 key 策略。
///
/// bind 本身不声明 key；它继承消费方请求 `dyn Trait` 时携带的 key，并据此派生待激活
/// 的 concrete provider token。这样 `#[inject(key = "red")] dyn Port` 可以匹配
/// `#[injectable(key = "red")] ConcretePort`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundKeyPolicy {
    InheritRequestedKey,
}

impl BoundKeyPolicy {
    /// 根据 trait 请求派生需要激活的 concrete token。
    pub fn concrete_identifier(
        self,
        requested: ServiceIdentifier,
        concrete_type: ServiceType,
    ) -> ServiceIdentifier {
        match self {
            Self::InheritRequestedKey => {
                ServiceIdentifier::new(requested.service_key, concrete_type)
            }
        }
    }
}

/// 当前链接单元内由 `#[bind]` 声明的 trait 绑定。
///
/// 与 provider 一样使用函数项，使每个 crate 都能在切片中构造 payload，且不要求应用
/// crate 直接依赖 `linkme`。
#[distributed_slice]
pub static REFLECTED_BINDINGS: [fn() -> TraitBinding] = [..];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registration::service_key::ServiceKey;

    struct Trait;
    struct Concrete;

    #[test]
    fn bound_key_policy_inherits_the_trait_request_key() {
        let requested = ServiceIdentifier::new(
            Some(ServiceKey::Named("red")),
            ServiceType::create::<Trait>(),
        );
        let concrete_type = ServiceType::create::<Concrete>();

        let resolved =
            BoundKeyPolicy::InheritRequestedKey.concrete_identifier(requested, concrete_type);

        assert_eq!(resolved.service_key, Some(ServiceKey::Named("red")));
        assert_eq!(resolved.service_type, ServiceType::create::<Concrete>());
    }
}
