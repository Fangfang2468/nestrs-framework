//! 工具链编译的完整、不可变服务执行计划。
//!
//! 生产入口通过 `plan` 装载编译结果，不再物化泛型、选择候选或执行图编译。
//! `tests/support/graph` 保存原算法与诊断参照，便于验证迁移前后的规则一致性。
//!
//! 本文件只定义生产执行计划，`plan` 负责装载目标程序中的 typed adapter 地址。
//! 这里的节点是已选定的服务执行单元，运行期一次 Transient 消费
//! 产生的实例/任务不等同于图节点。

#[cfg(test)]
#[path = "../../tests/support/graph/compiler.rs"]
mod compiler;
#[cfg(test)]
#[path = "../../tests/support/graph/diagnostics.rs"]
mod diagnostics;
#[cfg(test)]
#[path = "../../tests/support/graph/error.rs"]
mod error;
#[cfg(test)]
#[path = "../../tests/support/graph/names.rs"]
mod names;
pub(crate) mod plan;

use std::{collections::HashMap, sync::Arc};

use crate::{
    ServiceLifetime,
    activation::{
        InputPreparer, InputSlot, LazyInputPlan, LazyInputPreparer, ServiceProjector,
        adapter::CleanupHook,
    },
    service::{ServiceIdentifier, ServiceSource},
};

pub(crate) use crate::activation::adapter::Constructor;

#[cfg(test)]
pub(crate) use compiler::GraphCompiler;
#[cfg(test)]
pub(crate) use diagnostics::snapshot;
#[cfg(test)]
pub(crate) use error::{GraphDiagnostic, GraphDiagnosticKind, GraphError};

/// 冻结后节点数组的稳定下标。展开阶段完成排序之前的临时下标不能流入运行期。
pub(crate) type ProviderId = usize;

/// 一次构图的不可变执行计划。只有通过全部诊断检查的编译结果才会交给运行期。
#[derive(Debug)]
pub(crate) struct ValidatedGraph {
    pub(crate) nodes: Vec<CompiledNode>,
    pub(crate) routes: HashMap<ServiceIdentifier, RootRoute>,
    /// 依赖总在消费者之前，用于预热与已验证的 Scope 能力传播。
    pub(crate) topological_order: Vec<ProviderId>,
    /// 每个 Provider 的去重反向邻接表，按确定的 ProviderId 顺序排列。
    pub(crate) dependents: Vec<Vec<ProviderId>>,
}

/// 一个服务的构造计划；生命周期决定运行期有多少个实际 occurrence。
#[derive(Debug)]
pub(crate) struct CompiledNode {
    pub(crate) identifier: ServiceIdentifier,
    pub(crate) common: NodePolicy,
    /// 输入槽位完整保留，不能像拓扑边一样去重。
    pub(crate) dependencies: Vec<CompiledDependency>,
    pub(crate) constructor: Constructor,
    /// 本节点或其传递依赖包含 Scoped；据此拒绝 root 查询需要 Scope 的 Transient。
    pub(crate) requires_scope: bool,
}

/// 实例执行所需的固定策略。候选优先级等声明事实不会进入运行期节点。
#[derive(Debug, Clone, Copy)]
pub(crate) struct NodePolicy {
    pub(crate) lifetime: ServiceLifetime,
    /// None 继承当前 root/预热调用的默认值，Some 覆盖自主预热选择。
    pub(crate) lazy: Option<bool>,
    pub(crate) source: ServiceSource,
    pub(crate) cleanup: Option<CleanupHook>,
}

/// 已决定交付方式的一项构造输入，不再包含运行期候选选择或泛型展开逻辑。
#[derive(Debug, Clone)]
pub(crate) struct CompiledDependency {
    pub(crate) slot: InputSlot,
    /// 保留原始请求用于诊断，不能用选中的 concrete 身份覆盖 trait/key/缺席信息。
    pub(crate) requested: ServiceIdentifier,
    /// 原始请求的可选性仅用于诊断；运行期交付直接匹配 input，不再推导字段组合。
    /// 装配边界保证只有 optional 请求能形成 Absent，存在目标时适配器固定令牌形态。
    pub(crate) optional: bool,
    pub(crate) input: DependencyInput,
    pub(crate) label: Option<&'static str>,
}

/// 已冻结的一项执行选择。装配协议中的可选字段在这里收敛为互斥分支：
/// 缺席没有目标，立即输入必须有目标和准备函数，延迟输入必须有完整共享计划。
/// 运行期不再组合 target/lazy/lazy_plan 来判断是否调度或怎样交付。
#[derive(Debug, Clone)]
pub(crate) enum DependencyInput {
    Absent(AbsentInput),
    Immediate {
        target: ProviderId,
        prepare: InputPreparer,
    },
    Lazy {
        /// 全部消费者 occurrence 共享固定描述，各自的字段仍独立保存初始化状态。
        plan: Arc<LazyInputPlan>,
        prepare: LazyInputPreparer,
    },
}

/// 缺席输入仍要交付正确的 Rust 类型：Option<Injection<T>> 与
/// Option<LazyInjection<T>> 的 None 不可互换。这里仅保存相应准备函数，
/// 不创建目标任务、延迟计划或运行期请求句柄。
#[derive(Debug, Clone, Copy)]
pub(crate) enum AbsentInput {
    Immediate(InputPreparer),
    Lazy(LazyInputPreparer),
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
        matches!(self, Self::Lazy { .. } | Self::Absent(AbsentInput::Lazy(_)))
    }

    /// 只有实际存在延迟目标才有关联 owner 的必要；缺席输入不分配延迟状态。
    pub(crate) fn lazy_plan(&self) -> Option<&Arc<LazyInputPlan>> {
        match self {
            Self::Lazy { plan, .. } => Some(plan),
            Self::Absent(_) | Self::Immediate { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RootRoute {
    pub(crate) provider: ProviderId,
    /// trait 根与延迟访问共享直接投影能力；查询不经过构造输入的装箱与消费协议。
    /// concrete 根继续使用实例保存的准确类型地址。
    pub(crate) projection: Option<ServiceProjector>,
}

#[cfg(test)]
#[path = "../../tests/unit/graph/compiler.rs"]
mod tests;
