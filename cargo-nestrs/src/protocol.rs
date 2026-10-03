//! 工具内部的声明与计划协议，library/codegen 和 driver 各自私有地编入同一源文件。
//!
//! 本文件不引用 rustc 或 core，也不成为应用 API。两端仅交换生成的 Rust/metadata，
//! 不跨 crate 传递这里的 Rust 类型；真实来源、DefId 和函数签名仍由 driver 认证。
//! 两个编译目标使用不同子集，因此允许另一个目标专用的定义暂时未使用。
#![allow(dead_code)]

pub(crate) const REFLECTION_MODULE: &str = "__nestrs_reflect";
pub(crate) const COMPILER_KEY: &str = "CompilerKey";
pub(crate) const PROVIDER_DEFINITION: &str = "ProviderDefinition";
pub(crate) const PROVIDER_HELPER: &str = "provider_definition";
pub(crate) const PLAN_ENTRY: &str = "__nestrs_reflect_v1";
pub(crate) const ACTIVATION_ADAPTER: &str = "activation::adapter::ActivationAdapter";
pub(crate) const PROJECTION_ADAPTER: &str = "activation::adapter::ProjectionAdapter";

#[derive(Clone, Copy)]
pub(crate) enum Parameter {
    Type,
    Usize,
    U8,
    Bool,
}

#[derive(Clone, Copy)]
pub(crate) enum Inputs {
    None,
    Key,
    KeyLabel,
    Label,
}

#[derive(Clone, Copy)]
pub(crate) enum Marker {
    Provider,
    Dependency,
    Binding,
    AutomaticBinding,
    PlanProvider,
    PlanInput,
    PlanFactory,
    PlanOrigin,
    QueryRoot,
    QueryCall,
    QuerySummary,
}

impl Marker {
    const ALL: [Self; 11] = [
        Self::Provider,
        Self::Dependency,
        Self::Binding,
        Self::AutomaticBinding,
        Self::PlanProvider,
        Self::PlanInput,
        Self::PlanFactory,
        Self::PlanOrigin,
        Self::QueryRoot,
        Self::QueryCall,
        Self::QuerySummary,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Provider => "compiler_provider",
            Self::Dependency => "compiler_dependency",
            Self::Binding => "compiler_binding",
            Self::AutomaticBinding => "compiler_automatic_binding",
            Self::PlanProvider => "compiler_plan_provider",
            Self::PlanInput => "compiler_plan_input",
            Self::PlanFactory => "compiler_plan_factory",
            Self::PlanOrigin => "compiler_plan_origin",
            Self::QueryRoot => "compiler_query_root",
            Self::QueryCall => "compiler_query_call",
            Self::QuerySummary => "__nestrs_query_summary_v1",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|marker| marker.name() == name)
    }

    pub(crate) fn signature(self) -> (&'static [Parameter], Inputs) {
        use Parameter::{Bool, Type, U8, Usize};
        match self {
            Self::Provider => (&[Type], Inputs::Key),
            Self::Dependency => (&[Type, Usize], Inputs::None),
            Self::Binding | Self::AutomaticBinding => (&[Type, Type], Inputs::None),
            Self::PlanProvider => (&[Type, U8, Bool, U8], Inputs::Key),
            Self::PlanInput => (&[Type, Usize, Bool, Bool], Inputs::KeyLabel),
            Self::PlanFactory => (&[Bool], Inputs::None),
            Self::PlanOrigin => (&[U8, Usize], Inputs::Label),
            Self::QueryRoot | Self::QueryCall => (&[Type], Inputs::None),
            Self::QuerySummary => (&[], Inputs::None),
        }
    }
}

/// 诊断来源附着到同一个已认证描述回调；不会进入运行时计划 ABI。
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum OriginKind {
    Declaration = 0,
    Lifetime = 1,
    Primary = 2,
    ProviderKey = 3,
    InputKey = 4,
    InputTypeEnd = 5,
    ProviderTypeEnd = 6,
    Constructor = 7,
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Lifetime {
    Singleton = 0,
    Scoped = 1,
    Transient = 2,
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Initialization {
    Inherit = 0,
    Lazy = 1,
    Eager = 2,
}

impl Initialization {
    pub(crate) fn from_lazy(lazy: Option<bool>) -> Self {
        match lazy {
            None => Self::Inherit,
            Some(true) => Self::Lazy,
            Some(false) => Self::Eager,
        }
    }

    pub(crate) fn lazy(self) -> Option<bool> {
        match self {
            Self::Inherit => None,
            Self::Lazy => Some(true),
            Self::Eager => Some(false),
        }
    }
}

/// 对已认证 marker 的 const 参数解码；位置规则只在这里转换成具名策略。
pub(crate) struct ProviderPolicy {
    pub(crate) lifetime: Lifetime,
    pub(crate) primary: bool,
    pub(crate) initialization: Initialization,
}

impl ProviderPolicy {
    pub(crate) fn decode(constants: &[u128]) -> Result<Self, &'static str> {
        let [lifetime, primary, initialization] = constants else {
            return Err("DI provider metadata 版本不匹配");
        };
        let lifetime = match *lifetime {
            value if value == Lifetime::Singleton as u128 => Lifetime::Singleton,
            value if value == Lifetime::Scoped as u128 => Lifetime::Scoped,
            value if value == Lifetime::Transient as u128 => Lifetime::Transient,
            _ => return Err("无效的服务生命周期"),
        };
        let initialization = match *initialization {
            value if value == Initialization::Inherit as u128 => Initialization::Inherit,
            value if value == Initialization::Lazy as u128 => Initialization::Lazy,
            value if value == Initialization::Eager as u128 => Initialization::Eager,
            _ => return Err("无效的服务初始化策略（应为 inherit、lazy 或 eager）"),
        };
        Ok(Self {
            lifetime,
            primary: *primary != 0,
            initialization,
        })
    }
}

pub(crate) struct InputPolicy {
    pub(crate) slot: usize,
    pub(crate) optional: bool,
    pub(crate) lazy: bool,
}

impl InputPolicy {
    pub(crate) fn decode(constants: &[u128]) -> Result<Self, &'static str> {
        let [slot, optional, lazy] = constants else {
            return Err("DI input metadata 版本不匹配");
        };
        Ok(Self {
            slot: *slot as usize,
            optional: *optional != 0,
            lazy: *lazy != 0,
        })
    }
}

#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum KeyKind {
    Default = 0,
    Named = 1,
    Indexed = 2,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlanSink {
    Options,
    Binding,
    Provider,
    Input,
    TraitRoute,
    Order,
    Dependent,
}

impl PlanSink {
    const ALL: [Self; 7] = [
        Self::Options,
        Self::Binding,
        Self::Provider,
        Self::Input,
        Self::TraitRoute,
        Self::Order,
        Self::Dependent,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Options => "plan_set_options",
            Self::Binding => "plan_push_binding",
            Self::Provider => "plan_push_provider",
            Self::Input => "plan_set_input",
            Self::TraitRoute => "plan_push_trait_route",
            Self::Order => "plan_push_order",
            Self::Dependent => "plan_push_dependent",
        }
    }

    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sink| sink.name() == name)
    }
}

pub(crate) mod constructor {
    pub(crate) const METADATA: &str = "__NESTRS_CONSTRUCTOR";
    pub(crate) const ACTIVATE: &str = "__nestrs_constructor_activate";
    pub(crate) const DEPENDENCIES: &str = "__nestrs_constructor_dependencies";

    /// 宏写入、driver 在真实来源认证后读取；字段名及值保持现有 metadata 格式。
    #[derive(serde::Serialize, serde::Deserialize)]
    pub(crate) struct Metadata {
        pub(crate) method: String,
        pub(crate) result: bool,
    }
}
