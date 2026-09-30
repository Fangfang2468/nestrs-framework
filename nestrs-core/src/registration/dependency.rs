//! 依赖请求的注册 ABI。
//!
//! 一个依赖请求包含两个独立事实：值如何准备为构造输入槽位（[`Delivery`]），以及
//! 描述是否自带闭合 Provider 蓝图（[`ProviderSource`]）。trait 投影与泛型物化不能
//! 合并成同一个选择。图编译阶段还会核对 optional、投影形式与蓝图来源的一致性；
//! 描述结构本身不是“已验证”的标志。

use crate::{
    activation::{InputPreparer, InputSlot, LazyInputPreparer},
    registration::provider::Provider,
    service::ServiceIdentifier,
};

/// 一次字段或 factory 参数的依赖请求。
///
/// 字段与函数参数都通过该结构描述，因此 token、可选性、构造输入位置与交付方式在
/// class provider 和 factory provider 之间完全一致。
#[derive(Debug, Clone)]
pub struct DependencyRequest {
    /// 依赖在原始字段或参数声明中的零基位置。
    ///
    /// 对结构体字段，该位置包含 `#[value]` / 默认字段；它只服务于稳定诊断，不等同于
    /// 构造 ABI 的输入槽位。
    pub declaration_position: usize,

    /// 依赖在构造输入中的位置。
    pub input_slot: InputSlot,

    /// 查找依赖服务的 token。
    pub token: ServiceIdentifier,

    /// 缺失依赖时是否允许交付 `None`。
    pub optional: bool,

    /// 延迟注入槽位的类型化准备函数。None 表示必须在消费者构造前完成依赖。
    ///
    /// 延迟只改变激活时机，不能改变目标选择、缺失校验或生命周期规则；delivery
    /// 仍保留真实实例发布后需要使用的 concrete 地址或 trait 投影准备函数。
    pub lazy: Option<LazyInputPreparer>,

    /// 依赖诊断或元数据使用的可读标签。
    pub label: Option<&'static str>,

    /// 依赖值如何准备为构造输入槽位的载荷。
    pub delivery: Delivery,

    /// 描述是否携带消费点单态化的 Provider 蓝图。
    pub provider_source: ProviderSource,
}

/// 依赖值准备为构造输入槽位载荷的方式。
#[derive(Debug, Clone, Copy)]
pub enum Delivery {
    /// 明确要求 concrete 路由的内部输入协议，使用消费点单态化的准备函数。
    ///
    /// 图编译器必须拒绝把这个协议用于 trait 路由。现有普通宏生成使用 Selected
    /// 兼容类型别名和 ?Sized；Direct 继续用于明确 concrete 的内部描述与契约验证。
    Direct(InputPreparer),

    /// 源码路径可能是类型别名或 ?Sized 泛型参数，不能仅按拼写判断是不是 trait。
    /// concrete 路由使用准确类型地址，接口路由使用已选择 binding 的 preparer；
    /// 两者都依赖真实 typed adapter，不伪造胖指针元数据。
    Selected(InputPreparer),

    /// 必选 trait object：输入准备函数必须由匹配到的 binding 提供。
    ///
    /// 消费点只知道 trait，无法生成 concrete-to-trait 的 typed projector，因此这里
    /// 不携带直接 preparer；缺少匹配绑定就是缺失依赖。
    RequiresBinding,

    /// 可选 trait object：匹配到 binding 时使用它的投影函数，否则写入合法缺席。
    ///
    /// 携带的函数项只接受「依赖确实不存在」，拒绝把 concrete 薄指针伪造成
    /// trait-object 胖指针。
    RequiresBindingOrAbsent(InputPreparer),
}

/// 依赖声明是否自带闭合类型的 Provider 描述回调。
#[derive(Debug, Clone, Copy)]
pub enum ProviderSource {
    /// 消费点不携带物化回调。图仍可使用已注册 Provider 或 driver 汇总的被动蓝图；
    /// factory-only 和 trait 请求通常走这一分支。
    Registered,

    /// 缺少精确 type/key 的显式注册时，用消费点单态化的回调按需物化 Provider 描述。
    ///
    /// 开放泛型的类型实参不能从 `TypeId` 反推；例如 `Repository<User>` 必须由宏在
    /// 这个调用点嵌入一个返回闭合 provider 的 callback。
    Materialize(ClosedProviderCallback),
}

/// 已知闭合泛型服务转为 provider 的 callback。
pub type ClosedProviderCallback = fn() -> Provider;
