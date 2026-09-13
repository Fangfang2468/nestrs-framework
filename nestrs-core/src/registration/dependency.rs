//! 依赖请求的注册 ABI。
//!
//! 一个依赖请求由两个正交的事实组成：值如何写入构造输入槽位（[`Delivery`]），以及
//! provider 在解析期从哪里来（[`ProviderSource`]）。trait 投影与闭合泛型的按需物化
//! 分属这两个轴，因此不再混在同一组可选字段里，也不存在无法表达的非法组合。

use crate::{
    construction::{InputPosition, PrepareInput},
    registration::{provider::Provider, service_identifier::ServiceIdentifier},
};

/// 一次字段或 factory 参数的依赖请求。
///
/// 字段与函数参数都通过该结构描述，因此 token、可选性、构造输入位置与交付方式在
/// class provider 和 factory provider 之间完全一致。
#[derive(Debug, Clone, Copy)]
pub struct DependencyRequest {
    /// 依赖在原始字段或参数声明中的零基位置。
    ///
    /// 对结构体字段，该位置包含 `#[value]` / 默认字段；它只服务于稳定诊断，不等同于
    /// 构造 ABI 的输入槽位。
    pub declaration_position: usize,

    /// 依赖在构造输入中的位置。
    pub input_position: InputPosition,

    /// 查找依赖服务的 token。
    pub token: ServiceIdentifier,

    /// 缺失依赖时是否允许交付 `None`。
    pub optional: bool,

    /// 依赖诊断或元数据使用的可读标签。
    pub label: Option<&'static str>,

    /// 依赖值如何写入构造输入槽位。
    pub delivery: Delivery,

    /// provider 在解析期的来源。
    pub provider_source: ProviderSource,
}

/// 依赖值写入构造输入槽位的方式。
#[derive(Debug, Clone, Copy)]
pub enum Delivery {
    /// 消费点自己单态化的输入准备函数。
    ///
    /// concrete 依赖与闭合泛型服务都使用它；两者的差别只在
    /// [`ProviderSource`]，不在值的交付方式。
    Direct(PrepareInput),

    /// 必选 trait object：输入准备函数必须由匹配到的 `#[bind]` 提供。
    ///
    /// 消费点只知道 trait，无法生成 concrete-to-trait 的 typed projector，因此这里
    /// 不携带直接 preparer；缺少匹配绑定就是缺失依赖。
    RequiresBinding,

    /// 可选 trait object：匹配到 `#[bind]` 时使用它的投影函数，否则写入合法缺席。
    ///
    /// 携带的函数项只接受「依赖确实不存在」，拒绝把 concrete 薄指针伪造成
    /// trait-object 胖指针。
    RequiresBindingOrAbsent(PrepareInput),
}

/// 解析期为依赖寻找 provider 的方式。
#[derive(Debug, Clone, Copy)]
pub enum ProviderSource {
    /// 只接受显式注册的 provider。
    Registered,

    /// 缺少显式注册时，用消费点单态化的回调按需物化 provider。
    ///
    /// 开放泛型的类型实参不能从 `TypeId` 反推；例如 `Repository<User>` 必须由宏在
    /// 这个调用点嵌入一个返回闭合 provider 的 callback。
    Materialize(ClosedProviderCallback),
}

/// 已知闭合泛型服务转为 provider 的 callback。
pub type ClosedProviderCallback = fn() -> Provider;
