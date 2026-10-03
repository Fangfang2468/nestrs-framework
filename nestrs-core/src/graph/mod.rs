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
        InputPreparer, InputSlot, LazyInputPlan, LazyInputPreparer, adapter::CleanupHook,
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
    pub(crate) optional: bool,
    /// 延迟字段在消费者构造时交付句柄；目标边仍参与完整图验证与关闭排序。
    pub(crate) lazy: Option<LazyInputPreparer>,
    /// 每条存在目标的延迟边只有一份不可变描述；全部 root/scope/字段 occurrence 共享它。
    /// 缺席 optional 不创建描述，初始化接收端与结果缓存仍由每个字段分别拥有。
    pub(crate) lazy_plan: Option<Arc<LazyInputPlan>>,
    /// None 只在成功图中表示已确定缺席的 optional 输入。
    pub(crate) target: Option<ProviderId>,
    /// 目标发布后用此类型化函数准备槽位；target 为 None 时写入合法缺席值。
    pub(crate) prepare: InputPreparer,
    pub(crate) label: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RootRoute {
    pub(crate) provider: ProviderId,
    /// trait 根查询复用 binding 的必选输入投影，concrete 根使用实例的准确类型地址。
    pub(crate) projection: Option<InputPreparer>,
}

#[cfg(test)]
#[path = "../../tests/unit/graph/compiler.rs"]
mod tests;
