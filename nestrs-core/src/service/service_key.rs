//! 服务标识的可选限定符。
//!
//! 这是 `nestrs-core` 中唯一的运行时 `ServiceKey` 定义，同时用于公开门面与宏生成的
//! 注册 metadata。命名 key 自身拥有名称，因而不把宏期字符串、调用方借用字符串和
//! 后续运行时存储拆成不同的 key 类型。

/// 在同一服务类型命名空间内选择特定服务的限定符。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceKey {
    /// 由名称选择服务。
    Named(String),

    /// 由稳定编号选择服务。
    Indexed(usize),
}
