use linkme::distributed_slice;

use crate::{construction::PrepareInput, registration::service_type::ServiceType};

/// #[bind] 元数据，记录哪些服务绑定了哪些 trait。
#[derive(Debug, Clone, Copy)]
pub struct InterfaceBinding {
    /// 实现服务的具体类型。
    pub service_type: ServiceType,

    /// 服务导出的 trait 类型。
    pub trait_type: ServiceType,

    /// 将 concrete Arena 地址投影为该 trait 的必选 `Inject<dyn Trait>` 字段输入。
    ///
    /// 此函数项由 `#[bind] impl Trait for Concrete` 单态化生成，因而同时保有
    /// `Concrete` 的地址解释方式和 `dyn Trait` 的 vtable。运行时绝不从 `TypeId`
    /// 或薄指针猜测 trait object 的 metadata。
    #[doc(hidden)]
    pub prepare_required: PrepareInput,

    /// 将 concrete Arena 地址投影为该 trait 的可选字段输入。
    ///
    /// 当 bind 找到 concrete provider 时它构造 `Some(Inject<dyn Trait>)`；缺失的可选
    /// trait 依赖则由字段自身的专用空输入 adapter 构造 `None`。
    #[doc(hidden)]
    pub prepare_optional: PrepareInput,
}

/// #[bind] 元数据，记录哪些注册服务绑定了哪些 trait。
#[distributed_slice]
pub static REFLECT_METADATA_BIND: [fn() -> InterfaceBinding] = [..];
