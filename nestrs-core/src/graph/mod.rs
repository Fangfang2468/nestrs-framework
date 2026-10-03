//! 工具链编译的完整、不可变服务执行计划。
//!
//! 生产入口通过 `plan` 装载编译结果，不再物化泛型、选择候选或执行图编译。
//! `tests/support/graph` 只保存冻结计划的只读快照，不包含第二套图编译器。
//!
//! 本文件只定义生产执行计划，`plan` 负责装载目标程序中的 typed adapter 地址。
//! 这里的节点是已选定的服务执行单元，运行期一次 Transient 消费
//! 产生的实例/任务不等同于图节点。

#[cfg(test)]
#[path = "../../tests/support/graph/diagnostics.rs"]
mod diagnostics;
#[cfg(test)]
#[path = "../../tests/support/graph/names.rs"]
mod names;
pub(crate) mod plan;

use ahash::AHashMap;
use std::sync::Arc;

use crate::{
    ServiceLifetime,
    activation::{InputKind, InputSlot, LazyInputPlan, ServiceProjector, adapter::CleanupHook},
    service::{ServiceIdentifier, ServiceSource},
};

pub(crate) use crate::activation::adapter::Constructor;

#[cfg(test)]
pub(crate) use diagnostics::snapshot;

/// 冻结后节点数组的稳定下标。展开阶段完成排序之前的临时下标不能流入运行期。
pub(crate) type ProviderId = usize;

/// 一次构图的不可变执行计划。只有通过全部诊断检查的编译结果才会交给运行期。
#[derive(Debug)]
pub(crate) struct ValidatedGraph {
    /// 已选节点数组；路由和输入目标均使用此数组的稳定编号。
    pub(crate) nodes: Vec<CompiledNode>,

    /// 查询只需按完整服务身份查找；使用随机种子的 aHash，不把哈希顺序用于图语义。
    pub(crate) routes: AHashMap<ServiceIdentifier, RootRoute>,

    /// 依赖总在消费者之前，用于预热与已验证的 Scope 能力传播。
    pub(crate) topological_order: Vec<ProviderId>,

    /// 每个 Provider 的去重反向邻接表，按确定的 ProviderId 顺序排列。
    pub(crate) dependents: Vec<Vec<ProviderId>>,
}

/// 一个服务的构造计划；生命周期决定运行期有多少个实际 occurrence。
#[derive(Debug)]
pub(crate) struct CompiledNode {
    /// 当前 concrete 节点的准确类型与 key。
    pub(crate) identifier: ServiceIdentifier,

    /// 固定生命周期、预热覆盖、来源与清理策略。
    pub(crate) common: NodePolicy,

    /// 输入槽位完整保留，不能像拓扑边一样去重。
    pub(crate) dependencies: Vec<CompiledDependency>,

    /// 该节点创建实例使用的 Class 或 Factory 入口。
    pub(crate) constructor: Constructor,

    /// 本节点或其传递依赖包含 Scoped；据此拒绝 root 查询需要 Scope 的 Transient。
    pub(crate) requires_scope: bool,
}

/// 实例执行所需的固定策略。候选优先级等声明事实不会进入运行期节点。
#[derive(Debug, Clone, Copy)]
pub(crate) struct NodePolicy {
    /// 决定实际实例缓存和 owner 归属的生命周期。
    pub(crate) lifetime: ServiceLifetime,

    /// None 继承当前 root/预热调用的默认值，Some 覆盖自主预热选择。
    pub(crate) lazy: Option<bool>,

    /// 原始声明位置，供运行期失败诊断使用。
    pub(crate) source: ServiceSource,

    /// 成功实例逻辑关闭时执行的可选异步清理入口。
    pub(crate) cleanup: Option<CleanupHook>,
}

/// 已决定交付方式的一项构造输入，不再包含运行期候选选择或泛型展开逻辑。
#[derive(Debug, Clone)]
pub(crate) struct CompiledDependency {
    /// 当前输入的固定槽位编号。
    pub(crate) slot: InputSlot,

    /// 保留原始请求用于诊断，不能用选中的 concrete 身份覆盖 trait/key/缺席信息。
    pub(crate) requested: ServiceIdentifier,

    /// 装配时已与 adapter 的交付形态核对；缺席只能来自 optional 请求。
    /// 运行时据此验证准确令牌形态，不参与候选选择。
    pub(crate) optional: bool,

    /// 冻结后的缺席、立即或延迟输入动作。
    pub(crate) input: DependencyInput,

    /// 原字段或参数的可选名称，用于定位输入失败。
    pub(crate) label: Option<&'static str>,
}

impl CompiledDependency {
    /// 由已冻结的可选性与延迟形态计算准确的输入交付类别。
    pub(crate) fn kind(&self) -> InputKind {
        match (self.optional, self.input.is_lazy()) {
            (false, false) => InputKind::Required,
            (true, false) => InputKind::Optional,
            (false, true) => InputKind::LazyRequired,
            (true, true) => InputKind::LazyOptional,
        }
    }
}

/// 已冻结的一项执行选择。装配协议中的可选字段在这里收敛为互斥分支：
/// 缺席没有目标，立即输入必须有目标和投影函数，延迟输入必须有完整共享计划。
/// 运行期不再组合 target/lazy/lazy_plan 来判断是否调度或怎样交付。
#[derive(Debug, Clone)]
pub(crate) enum DependencyInput {
    /// 可选目标缺席，仍保留准确的普通或延迟交付形态。
    Absent(AbsentInput),

    /// 必须先构造目标，再按已选投影向消费者交付普通输入。
    Immediate {
        /// 已选目标节点编号，不在运行时重新解析候选。
        target: ProviderId,

        /// 从已选目标实例交付所需类型的投影函数。
        project: ServiceProjector,
    },

    /// 将共享描述交给延迟句柄，目标不作为消费者构造的就绪前提。
    Lazy {
        /// 全部消费者 occurrence 共享固定描述，各自的字段仍独立保存初始化状态。
        plan: Arc<LazyInputPlan>,
    },
}

/// 缺席输入仍要交付正确的 Rust 类型：`Option<Injection<T>>` 与
/// `Option<LazyInjection<T>>` 的 None 不可互换。这里保留准确交付类别，
/// 不创建目标任务、延迟计划或运行期请求句柄。
#[derive(Debug, Clone, Copy)]
pub(crate) enum AbsentInput {
    /// 交付准确的 `Option<Injection<T>>::None`。
    Immediate,

    /// 交付准确的 `Option<LazyInjection<T>>::None`，不分配延迟状态。
    Lazy,
}

impl DependencyInput {
    /// 完整静态依赖关系用于生命周期诊断与关闭排序，必须包含延迟目标。
    /// 激活前置依赖只匹配 Immediate，不能直接用此方法展开构造任务。
    pub(crate) fn target(&self) -> Option<ProviderId> {
        match self {
            Self::Absent(_) => None,
            Self::Immediate { target, .. } => Some(*target),
            Self::Lazy { plan, .. } => Some(plan.provider),
        }
    }

    /// 保留延迟声明的诊断形态，含没有候选的 optional 延迟输入。
    pub(crate) fn is_lazy(&self) -> bool {
        matches!(self, Self::Lazy { .. } | Self::Absent(AbsentInput::Lazy))
    }

    /// 只有实际存在延迟目标才有关联 owner 的必要；缺席输入不分配延迟状态。
    #[cfg(test)]
    pub(crate) fn lazy_plan(&self) -> Option<&Arc<LazyInputPlan>> {
        match self {
            Self::Lazy { plan, .. } => Some(plan),
            Self::Absent(_) | Self::Immediate { .. } => None,
        }
    }
}

/// 一次公开查询已选定的节点及可选 trait 投影能力。
#[derive(Debug, Clone, Copy)]
pub(crate) struct RootRoute {
    /// 冻结计划中的 provider 节点编号。
    pub(crate) provider: ProviderId,

    /// trait 根与延迟访问共享直接投影能力；查询不经过构造输入的装箱与消费协议。
    /// concrete 根从最终实例记录的共享借用恢复准确类型地址。
    pub(crate) projection: Option<ServiceProjector>,
}
