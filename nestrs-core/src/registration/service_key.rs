// use std::hash::Hash;

// pub trait ServiceKey: Eq + Hash + 'static {}

// impl<T> ServiceKey for T
// where
//     T: Eq + Hash + 'static,
// {
    
// }



#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceKey {
    /// 同一服务类型命名空间中的静态名称。
    Named(&'static str),
    /// 同一服务类型命名空间中的稳定编号。
    Indexed(usize),
}

/// 将公开限定符输入转换为 [`ServiceKey`]。
///
/// 该转换只建立注册元数据，不会执行运行时解析。
pub trait IntoServiceKey {
    /// 转换为注册输入使用的 [`ServiceKey`]。
    fn into_service_key(self) -> ServiceKey;
}

impl IntoServiceKey for &'static str {
    fn into_service_key(self) -> ServiceKey {
        ServiceKey::Named(self)
    }
}

impl IntoServiceKey for usize {
    fn into_service_key(self) -> ServiceKey {
        ServiceKey::Indexed(self)
    }
}