use std::any::{TypeId, type_name};

use crate::registration::injectable::Injectable;

/// 对 TypeId 的一薄封装，为注册的服务保存更多的相关类型信息（主要是方便更智能的报错或提示）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceType {
    /// 类型ID
    pub type_id: TypeId,

    /// 类型名
    pub name: &'static str,
}


impl ServiceType {

    /// 创建方法 
    pub fn create<S>() -> Self
    where
        S: Injectable + ?Sized,
    {
        Self {
            type_id: TypeId::of::<S>(),
            name: type_name::<S>(),
        }
    }

    /// 获取类型短名称
    /// 
    /// ``` rust
    /// use nestrs_injection::registration::service_type::{ServiceType};
    /// 
    /// struct UserService {
    ///    name: String,
    /// }
    ///
    /// let service_type = ServiceType::create::<UserService>();
    /// println!("{}", service_type.short_name());   // 👈 打印： UserService
    /// ```
    pub fn short_name(&self) -> &'static str {
        self.name.rsplit("::").next().unwrap_or(self.name)
    }
}


