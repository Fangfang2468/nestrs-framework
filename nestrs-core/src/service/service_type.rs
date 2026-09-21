use std::any::{TypeId, type_name};

use crate::service::Injectable;

/// 宏注册元数据中的 Rust 类型 token。
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
