use linkme::distributed_slice;

use crate::registration::service_type::ServiceType;

/// #[bind] 元数据，记录哪些服务绑定了哪些 trait。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterfaceBinding {
    /// 实现服务的具体类型。
    pub service_type: ServiceType,

    /// 服务导出的 trait 类型。
    pub trait_type: ServiceType,
}

/// #[bind] 元数据，记录哪些注册服务绑定了哪些 trait。
#[distributed_slice]
pub static REFLECT_METADATA_BIND: [fn() -> InterfaceBinding] = [..];
