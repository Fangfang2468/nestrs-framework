use std::any::{TypeId, type_name};

use crate::service::Injectable;

/// 声明、路由和构造检查共同使用的真实 Rust 类型身份。
///
/// TypeId 负责匹配；name 用于诊断，不能通过同名字符串把不同 crate 的类型合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceType {
    pub type_id: TypeId,
    pub name: &'static str,
}

impl ServiceType {
    pub fn create<S>() -> Self
    where
        S: Injectable + ?Sized,
    {
        Self {
            type_id: TypeId::of::<S>(),
            name: type_name::<S>(),
        }
    }
}
