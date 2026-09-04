use linkme::distributed_slice;

use crate::{
    construction::{Constructor, PrepareInput},
    lifetime::Lifetime,
    registration::{
        injectable::Injectable, service_identifier::ServiceIdentifier,
        service_source::ServiceSource,
    },
};

/// `#[injectable]` 为一个具体服务类型生成的构造蓝图。
///
/// 这个 trait 与 [`Injectable`] 的含义不同：后者是所有满足 `Send + Sync + 'static`
/// 的类型自动获得的标记，而本 trait 只能由 `#[injectable]` 宏为实际 provider 生成。
/// 对泛型 provider 而言，调用方必须已经给出闭合类型，例如
/// `Repository<UserEntity>`；编译器会据此单态化 [`Self::component`]。
///
/// 它是宏与 composition runtime 之间的隐藏 ABI，不是业务代码的注册入口。
#[doc(hidden)]
pub trait ComponentDefinition: Injectable {
    /// 返回 `Self` 这个已闭合服务类型的完整 component 定义。
    fn component() -> StructComponent
    where
        Self: Sized;
}

/// 可嵌入字段依赖元数据的、已单态化 component 定义回调。
///
/// 使用函数指针而不是 trait object，使宏可以把
/// [`component_definition::<S>`] 作为没有捕获的静态 callback 保存下来。运行时无需、
/// 也不能从 [`std::any::TypeId`] 反推泛型参数。
#[doc(hidden)]
pub type ComponentDefinitionCallback = fn() -> StructComponent;

/// 将一个已知的 [`ComponentDefinition`] 转换为 component 生成 callback 可调用的形式。
///
/// 宏应以 `component_definition::<ClosedService>` 的单态化函数项填充
/// [`FieldInjection::component_definition`]；例如
/// `component_definition::<Repository<UserEntity>>`。
#[doc(hidden)]
pub fn component_definition<S>() -> StructComponent
where
    S: ComponentDefinition,
{
    S::component()
}

#[derive(Debug, Clone)]
pub struct FieldInjection {
    /// 字段在结构体声明中的零基位置。
    ///
    /// 该位置包含 `#[value(...)]` 和默认初始化字段，因此只用于保留原始字段
    /// 布局和诊断定位；它不是构造 ABI 的输入槽位。
    pub field_index: usize,

    /// 注入字段的名称。
    ///
    /// 元组结构体没有 Rust 字段标识符，故使用 `None`，避免把 `"0"` 之类的
    /// 人造名称泄漏为反射元数据。
    pub field_name: Option<&'static str>,

    /// 依赖在组件构造输入中的零基位置。
    ///
    /// 仅对注入字段连续编号；`#[value(...)]` 和默认初始化字段不占用此位置。
    /// 它必须与宏生成构造器的输入顺序一致。
    pub dependency_position: usize,

    /// 注入的服务的 [`ServiceIdentifier`]。
    pub service_identifier: ServiceIdentifier,

    /// 此字段请求的是 concrete service、trait object，还是当前 ABI 无法准备的类型。
    ///
    /// `dyn Trait` 不能当作普通 concrete 地址转换：它需要 `#[bind]` 中保存的类型化
    /// vtable projector。将这个事实写入 metadata 后，实例化器无需通过 `TypeId` 猜测
    /// 原始字段语法。
    pub target: FieldInjectionTarget,

    /// 当精确服务注册不存在时，用于具体化此字段所需服务的 component 蓝图。
    ///
    /// 它只记录宏在这个注入点已经看到的闭合类型，例如
    /// `Repository<UserEntity>`；并不表示运行时可以从任意 `TypeId` 反射出
    /// `Repository<_>`。普通依赖以及无法由 `#[injectable]` 生成蓝图的目标保持
    /// `None`。
    pub component_definition: Option<ComponentDefinitionCallback>,

    /// 将 Arena 中已查到的擦除服务恢复为此字段精确 `Inject<T>` 的单态化输入 adapter。
    ///
    /// runtime 只传递稳定地址；`T` 由 `#[injectable]` 宏生成的函数项保留，因此不会
    /// 发生基于 `TypeId` 的反推。`dyn Trait` 的 concrete-to-trait 投影由匹配的
    /// [`crate::metadata::impl_bind::InterfaceBinding`] 提供；可选 trait 没有 bind 时，
    /// 此处保存专用的空输入 adapter 以构造 `None`。
    pub prepare_input: Option<PrepareInput>,

    /// 是否是可选注入
    pub optional: bool,
}

/// `#[inject]` 字段的运行时地址交付类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldInjectionTarget {
    /// 字段服务类型与 Arena 已提交 concrete 服务类型相同。
    Concrete,
    /// 字段是 `dyn Trait`，必须经 `#[bind]` 的 concrete-to-trait projector 交付。
    TraitObject,
    /// 当前稳定地址 ABI 尚不能表示的字段类型。
    Unsupported,
}

#[derive(Debug, Clone)]
pub struct StructComponent {
    /// 此 provider 导出的服务身份（类型及可选 key）。
    pub service_identifier: ServiceIdentifier,

    /// 此 provider 的生命周期。
    pub lifetime: Lifetime,

    /// 服务的字段注入
    pub field_injections: Vec<FieldInjection>,

    /// 此 provider 的结构化构造 adapter。
    ///
    /// 宏生成的函数在其匿名 `const` 词法作用域中从预绑定输入取出 `Inject<T>`，
    /// 构造 concrete service 后以 [`crate::construction::ErasedService`] 返回。这样
    /// callback 不会成为用户可调用的 inherent method。
    pub constructor: Constructor,

    /// 当多个 [`ServiceIdentifier`] 能匹配当前服务时（一般常见使用 trait 标注字段类型时的注入），该服务是否是首选服务
    pub primary: bool,

    /// 服务的定义源
    pub source: ServiceSource,
}

/// 记录使用 `#[injectable]` 注册的服务的元数据。
#[distributed_slice]
pub static REFLECT_METADATA_INJECTABLE: [fn() -> StructComponent] = [..];
