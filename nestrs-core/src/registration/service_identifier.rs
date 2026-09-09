use crate::registration::{
    service_key::ServiceKey, service_type::ServiceType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceIdentifier {
    /// 服务的 Key
    pub service_key: Option<ServiceKey>,

    /// 服务类型
    pub service_type: ServiceType,
}

impl ServiceIdentifier {
    pub fn new(service_key: Option<ServiceKey>, service_type: ServiceType) -> Self {
        Self {
            service_key,
            service_type,
        }
    }
}

impl From<ServiceType> for ServiceIdentifier {
    fn from(value: ServiceType) -> Self {
        Self::new(None, value)
    }
}