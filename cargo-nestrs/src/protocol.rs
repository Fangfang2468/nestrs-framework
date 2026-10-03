//! 工具内部的声明与计划协议，library/codegen 和 driver 各自私有地编入同一源文件。
//!
//! 本文件不引用 rustc 或 core，也不成为应用 API。两端仅交换生成的 Rust/metadata，
//! 不跨 crate 传递这里的 Rust 类型；真实来源、DefId 和函数签名仍由 driver 认证。
//! 两个编译目标使用不同子集，因此允许另一个目标专用的定义暂时未使用。
#![allow(dead_code)]

/// 业务声明中承载工具 marker 的私有反射模块名称。
pub(crate) const REFLECTION_MODULE: &str = "__nestrs_reflect";

/// 声明 metadata 中用于表达服务 key 的工具内部类型名。
pub(crate) const COMPILER_KEY: &str = "CompilerKey";

/// 闭合 provider 描述所实现的工具内部 trait 名称。
pub(crate) const PROVIDER_DEFINITION: &str = "ProviderDefinition";

/// 供 driver 识别 provider 定义能力的描述函数名。
pub(crate) const PROVIDER_HELPER: &str = "provider_definition";

/// 最终 binary/test 的唯一执行计划入口符号；入口 v2、Options sink v3 与展示 JSON 分别版本化。
pub(crate) const PLAN_ENTRY: &str = "__nestrs_reflect_v2";

/// core 内部激活适配能力的真实路径，由 driver 校验类型身份。
pub(crate) const ACTIVATION_ADAPTER: &str = "activation::adapter::ActivationAdapter";

/// core 内部 concrete 到 trait 投影能力的真实路径。
pub(crate) const PROJECTION_ADAPTER: &str = "activation::adapter::ProjectionAdapter";

/// marker 泛型参数的种类；两端据此生成或认证签名。
#[derive(Clone, Copy)]
pub(crate) enum Parameter {
    /// 真实类型参数，最终由 rustc 提供类型身份。
    Type,

    /// 槽位或标签使用的 usize 常量参数。
    Usize,

    /// 紧凑策略标签使用的 u8 常量参数。
    U8,

    /// 可选、延迟等开关使用的布尔常量参数。
    Bool,
}

/// marker 普通参数的形状，用于保留 key 和诊断标签。
#[derive(Clone, Copy)]
pub(crate) enum Inputs {
    /// 没有普通运行值参数。
    None,

    /// 只携带服务 key。
    Key,

    /// 同时携带 key 与输入标签。
    KeyLabel,

    /// 只携带诊断来源标签。
    Label,
}

/// 声明、输入及查询摘要的工具内部标记种类。
#[derive(Clone, Copy)]
pub(crate) enum Marker {
    /// 声明一个 provider 的类型与 key。
    Provider,

    /// 关联 provider 的输入类型和槽位。
    Dependency,

    /// 显式 concrete/trait 投影声明。
    Binding,

    /// 由工具收集的自动投影能力。
    AutomaticBinding,

    /// 携带 provider 的生命周期、primary 和初始化策略。
    PlanProvider,

    /// 携带一个输入槽位的完整交付策略。
    PlanInput,

    /// 记录构造来源是否属于 factory。
    PlanFactory,

    /// 附加供诊断定位的来源类别与槽位。
    PlanOrigin,

    /// 记录已闭合的服务查询需求。
    QueryRoot,

    /// 记录尚需沿真实调用继续闭合的查询相关类型。
    QueryCall,

    /// 保留 concrete 到动态接口转换关系。
    QueryUnsize,

    /// 标记可跨 crate 解码的查询摘要入口。
    QuerySummary,
}

impl Marker {
    /// 当前协议允许识别的完整符号集合。
    const ALL: [Self; 12] = [
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
        Self::QueryUnsize,
        Self::QuerySummary,
    ];

    /// 返回双方约定的内部符号名；名称匹配之外仍需认证真实来源。
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
            Self::QueryUnsize => "compiler_query_unsize",
            Self::QuerySummary => "__nestrs_query_summary_v1",
        }
    }

    /// 只识别当前协议登记的内部符号，不接受相似名称。
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|marker| marker.name() == name)
    }

    /// 给出 marker 的泛型参数和普通输入形状，供生成与认证共用。
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
            Self::QueryUnsize => (&[Type, Type], Inputs::None),
            Self::QuerySummary => (&[], Inputs::None),
        }
    }
}

/// 诊断来源附着到同一个已认证描述回调；不会进入运行时计划 ABI。
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum OriginKind {
    /// 服务声明本身的来源。
    Declaration = 0,

    /// 服务生命周期属性的来源。
    Lifetime = 1,

    /// primary 选择属性的来源。
    Primary = 2,

    /// provider key 字面量的来源。
    ProviderKey = 3,

    /// 输入 key 字面量的来源。
    InputKey = 4,

    /// 输入类型末端位置，供诊断标注或建议使用。
    InputTypeEnd = 5,

    /// provider 类型末端位置。
    ProviderTypeEnd = 6,

    /// 显式构造方法选择的来源。
    Constructor = 7,
}

/// provider 生命周期的 metadata 数值编码，与运行期 Rust 类型解耦。
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Lifetime {
    /// 同一个 root 共享实例。
    Singleton = 0,

    /// 同一个 scope 共享实例。
    Scoped = 1,

    /// 每个消费 occurrence 独立构造。
    Transient = 2,
}

/// owner 创建期间是否自主初始化服务的三态编码，不表示字段延迟注入。
#[derive(Clone, Copy)]
#[repr(u8)]
pub(crate) enum Initialization {
    /// Singleton 继承 root 默认，Scoped 继承 scope 默认；Transient 不自主初始化。
    Inherit = 0,

    /// 不作为创建阶段的自主初始化入口，普通依赖仍可触发构造。
    Lazy = 1,

    /// 创建所属 root/scope 时显式选为初始化入口；Transient 仍按消费构造。
    Eager = 2,
}

impl Initialization {
    /// 将声明上的可选布尔策略转换为稳定的 metadata 编码。
    pub(crate) fn from_lazy(lazy: Option<bool>) -> Self {
        match lazy {
            None => Self::Inherit,
            Some(true) => Self::Lazy,
            Some(false) => Self::Eager,
        }
    }

    /// 还原声明语义，None 保留继承所属 owner 默认策略的含义。
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
    /// 已验证的 provider 生命周期。
    pub(crate) lifetime: Lifetime,

    /// 同 key 的 trait 多候选选择中是否拥有 primary 优先级。
    pub(crate) primary: bool,

    /// 服务级初始化策略，允许继承所属 owner 的默认值。
    pub(crate) initialization: Initialization,
}

impl ProviderPolicy {
    /// 核对常量数量与枚举编码，再构造 provider 策略；未知编码明确拒绝。
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

/// 已认证输入 marker 中的槽位、缺席和延迟交付策略。
pub(crate) struct InputPolicy {
    /// 输入在构造签名中的零起始位置。
    pub(crate) slot: usize,

    /// 缺少候选时是否允许交付 None；不隐藏其他图错误。
    pub(crate) optional: bool,

    /// 是否交付延迟句柄而不把目标作为构造就绪前提。
    pub(crate) lazy: bool,
}

impl InputPolicy {
    /// 核对输入 marker 常量数量，并恢复原槽位与交付修饰。
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

/// key 的 metadata 标签；默认、字符串和整数之间保持严格区分。
#[derive(Clone, Copy)]
#[repr(usize)]
pub(crate) enum KeyKind {
    /// 未配置 key，不能与 named 或 indexed 混同。
    Default = 0,

    /// 由字符串区分的 key。
    Named = 1,

    /// 由整数区分的 key。
    Indexed = 2,
}

/// 已冻结执行计划向 core 私有装配入口写入的数据类别。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PlanSink {
    /// 经 v3 sink 写入 root/scope 独立初始化默认值与共享构造并发上限。
    Options,

    /// 写入已启用的 concrete/trait 投影能力。
    Binding,

    /// 写入已验证的 provider 执行描述。
    Provider,

    /// 写入一个 provider 已选定的输入动作。
    Input,

    /// 写入 trait 查询到 provider 和投影的冻结路由。
    TraitRoute,

    /// 写入依赖优先的拓扑顺序。
    Order,

    /// 写入关闭和生命周期使用的反向依赖关系。
    Dependent,
}

impl PlanSink {
    /// 当前协议允许识别的完整符号集合。
    const ALL: [Self; 7] = [
        Self::Options,
        Self::Binding,
        Self::Provider,
        Self::Input,
        Self::TraitRoute,
        Self::Order,
        Self::Dependent,
    ];

    /// 返回双方约定的内部符号名；名称匹配之外仍需认证真实来源。
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Options => "plan_set_options_v3",
            Self::Binding => "plan_push_binding",
            Self::Provider => "plan_push_provider",
            Self::Input => "plan_set_input",
            Self::TraitRoute => "plan_push_trait_route",
            Self::Order => "plan_push_order",
            Self::Dependent => "plan_push_dependent",
        }
    }

    /// 只识别当前协议登记的内部符号，不接受相似名称。
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sink| sink.name() == name)
    }
}

/// 显式 constructor 在声明桥接与 driver 之间共享的私有名称和载荷。
pub(crate) mod constructor {
    /// 显式构造方法携带的工具私有 metadata 项名称。
    pub(crate) const METADATA: &str = "__NESTRS_CONSTRUCTOR";

    /// 显式构造激活辅助项的协议基名。
    pub(crate) const ACTIVATE: &str = "__nestrs_constructor_activate";

    /// 显式构造依赖描述辅助项的协议基名。
    pub(crate) const DEPENDENCIES: &str = "__nestrs_constructor_dependencies";

    /// 宏写入、driver 在真实来源认证后读取；字段名及值保持现有 metadata 格式。
    #[derive(serde::Serialize, serde::Deserialize)]
    pub(crate) struct Metadata {
        /// 用户选定的真实构造方法名称。
        pub(crate) method: String,

        /// 构造返回是否为 Result，需要生成错误转换。
        pub(crate) result: bool,

        /// 原始方法输入 token，用于后续解析和语义关联。
        pub(crate) input: String,
    }
}
