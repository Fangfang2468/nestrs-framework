use crate::{
    lifetime::Lifetime, registration::{injectable::Injectable, service_key::ServiceKey, service_source::ServiceSource, service_type::ServiceType},
};

pub enum ServiceImplementation {
    /// 实例（直接提供服务的具体类型）
    Instance(Box<dyn Injectable>),

    /// 构造工厂函数（传入一个函数用来实例化服务，类型自行）
    ConstructFactory(fn()),
}

/// 描述注册的服务
pub struct ServiceDescriptor {
    /// 注册服务的生命周期
    pub lifetime: Lifetime,

    /// 服务的 key
    pub service_key: Option<ServiceKey>,

    /// 当前注册的服务类型（只能是 struct 的类型）
    pub service_type: ServiceType,

    /// 实现服务实例的方式
    pub implementation: ServiceImplementation,

    /// 是否为默认实现
    pub primary: bool,

    /// 注册服务的声明来源，用于诊断信息或提示信息
    pub source: ServiceSource,
}

impl ServiceDescriptor {
    
    /// 创建无 Key 的服务描述
    #[track_caller]
    pub fn create(
        service_type: ServiceType,
        implementation: ServiceImplementation,
        lifetime: Lifetime,
        primary: bool
    ) -> Self {
        Self {
            lifetime,
            service_key: None,
            service_type,
            implementation,
            primary,
            source: ServiceSource::caller()
        }
    }

    /// 创建有 Key 的服务描述
    #[track_caller]
    pub fn create_keyed(
        service_type: ServiceType,
        service_key: ServiceKey,
        implementation: ServiceImplementation,
        lifetime: Lifetime,
        primary: bool
    ) -> Self {
        Self {
            lifetime,
            service_key: Some(service_key),
            service_type,
            implementation,
            primary,
            source: ServiceSource::caller()
        }
    }

    /// 创建瞬时服务描述符
    pub fn transient<IService>(
        implementation: ServiceImplementation,
        primary: bool, 
    ) -> Self
    where 
        IService: Injectable + ?Sized
    {
        Self::create(
            ServiceType::create::<IService>(),
            implementation,
            Lifetime::Transient,
            primary
        )
    }

    /// 创建有key的瞬时服务描述符
    pub fn keyed_transient<IService>(
        service_key: ServiceKey,
        implementation: ServiceImplementation,
        primary: bool, 
    ) -> Self 
    where 
        IService: Injectable + ?Sized
    {
        Self::create_keyed(
            ServiceType::create::<IService>(),
            service_key,
            implementation,
            Lifetime::Transient,
            primary
        )
    }


    /// 创建作用域服务描述符
    pub fn scope<IService>(
        implementation: ServiceImplementation,
        primary: bool, 
    ) -> Self
    where 
        IService: Injectable + ?Sized
    {
        Self::create(
            ServiceType::create::<IService>(),
            implementation,
            Lifetime::Scoped,
            primary
        )
    }

    /// 创建有key的作用域服务描述符
    pub fn keyed_scope<IService>(
        service_key: ServiceKey,
        implementation: ServiceImplementation,
        primary: bool, 
    ) -> Self 
    where 
        IService: Injectable + ?Sized
    {
        Self::create_keyed(
            ServiceType::create::<IService>(),
            service_key,
            implementation,
            Lifetime::Scoped,
            primary
        )
    }


    /// 创建单例服务描述符
    pub fn singleton<IService>(
        implementation: ServiceImplementation,
        primary: bool, 
    ) -> Self
    where 
        IService: Injectable + ?Sized
    {
        Self::create(
            ServiceType::create::<IService>(),
            implementation,
            Lifetime::Singleton,
            primary
        )
    }

    /// 创建有key的单例服务描述符
    pub fn keyed_singleton<IService>(
        service_key: ServiceKey,
        implementation: ServiceImplementation,
        primary: bool, 
    ) -> Self 
    where 
        IService: Injectable + ?Sized
    {
        Self::create_keyed(
            ServiceType::create::<IService>(),
            service_key,
            implementation,
            Lifetime::Singleton,
            primary
        )
    }
}

impl ServiceDescriptor {
    /// 判断服务是否有设置 key
    pub fn is_keyed_service(&self) -> bool {
        self.service_key.is_some()
    }
}
