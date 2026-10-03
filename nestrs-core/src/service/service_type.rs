use std::any::{TypeId, type_name};

use crate::service::Injectable;

/// 声明、路由和构造检查共同使用的真实 Rust 类型身份。
///
/// 派生的比较与哈希同时包含 TypeId 和 name；标准构造入口从同一类型生成两者，
/// name 另用于诊断。不能仅凭同名字符串把不同 crate 的类型合并。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServiceType {
    /// 用于准确相等性和路由查找的目标端 TypeId。
    pub type_id: TypeId,

    /// 由 type_name 取得的名称，用于诊断，并参与派生的比较与哈希。
    pub name: &'static str,
}

impl ServiceType {
    /// 从同一个 Rust 类型取得 TypeId 与名称，供实例、路由和投影一致地核对。
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
