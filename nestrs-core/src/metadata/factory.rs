use linkme::distributed_slice;

use crate::registration::{
    service_identifier::ServiceIdentifier, service_source::ServiceSource, service_type::ServiceType,
};


#[derive(Debug, Clone)]
pub struct FactoryParameterInjection {
    /// 参数在函数签名中的位置
    pub parameter_index: usize,

    /// 注入的服务的 [`ServiceIdentifier`]。
    pub service_identifier: ServiceIdentifier,

    /// 是否是可选注入
    pub optional: bool,
}


#[derive(Debug, Clone)]
pub struct FactoryComponent {
    /// 服务的类型
    pub service_type: ServiceType,

    /// 服务的字段注入
    pub parameter_injections: Vec<FactoryParameterInjection>,

    /// 当多个 [`ServiceIdentifier`] 能匹配当前服务时（一般常见使用 trait 标注字段类型时的注入），该服务是否是首选服务
    pub primary: bool,

    /// 服务的定义源
    pub source: ServiceSource,
}

/// 记录使用 `#[injectable]` 注册的服务的元数据。
#[distributed_slice]
pub static REFLECT_METADATA_FACTORY: [fn() -> FactoryComponent] = [..];
