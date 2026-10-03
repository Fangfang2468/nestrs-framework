use crate::service::{ServiceKey, ServiceType};

/// 类型与 key 共同组成的精确查询身份。
/// None 表示未指定 key，不能回退到任意命名或编号注册。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceIdentifier {
    /// 服务的 Key
    pub service_key: Option<ServiceKey>,

    /// 服务类型
    pub service_type: ServiceType,
}

impl ServiceIdentifier {
    /// 组合准确服务类型与完整 key，作为固定查询路由的身份。
    pub fn new(service_key: Option<ServiceKey>, service_type: ServiceType) -> Self {
        Self {
            service_key,
            service_type,
        }
    }
}

impl From<ServiceType> for ServiceIdentifier {
    /// 为指定 Rust 类型建立默认 key 的服务身份。
    fn from(value: ServiceType) -> Self {
        Self::new(None, value)
    }
}
