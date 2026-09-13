//! 依赖请求的唯一语法前端。
//!
//! `#[injectable]` 字段与 `#[factory]` 参数共享同一套 provider 请求语法：
//! `#[inject]` / `#[inject(key = ...)]` 属性、唯一允许的 `Option<T>` 可选形态，
//! 以及可以被容器表达的精确服务类型。两个宏入口只保留各自的措辞与后续构造代码
//! 形状，语法本身只在这里实现一次。

pub(crate) mod parse;
pub(crate) mod shape;

pub(crate) use parse::inject_key;
pub(crate) use shape::{
    DependencyShape, FACTORY_MESSAGES, INJECTABLE_MESSAGES, classify, requires_materialization,
    split_optional, unparenthesized_type,
};

use crate::injection::attrs::service_key::ServiceKey;

/// 一个依赖请求的宏期事实。
///
/// 它与 `nestrs_core::registration::dependency::DependencyRequest` 一一对应：这里是
/// 语法层事实，后者是写进 provider 注册 ABI 的运行时描述。`#[inject]` 字段与
/// factory 参数都先归一到这个形状，再共享同一套渲染逻辑。
#[derive(Clone, Debug)]
pub(crate) struct DependencyRequest {
    /// 依赖在字段或参数声明中的零基位置。
    ///
    /// 对结构体字段，该位置包含 `#[value]` 与默认字段；它只服务于稳定诊断，不等同
    /// 于构造 ABI 的输入槽位。
    pub(crate) declaration_position: usize,

    /// 依赖在构造输入中的位置。
    pub(crate) input_position: usize,

    /// 请求的服务类型；已剥离最外层 `Option` 与多余括号。
    pub(crate) service_type: zyn::syn::Type,

    /// 静态服务限定符。
    pub(crate) key: Option<ServiceKey>,

    /// 缺失依赖时是否允许交付 `None`。
    pub(crate) optional: bool,

    /// 具名字段或参数的名称；元组字段为 `None`。
    pub(crate) label: Option<zyn::syn::Ident>,
}
