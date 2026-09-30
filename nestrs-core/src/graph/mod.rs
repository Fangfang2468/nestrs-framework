//! 在执行任何用户构造代码之前编译完整、不可变的服务图。
//!
//! 元数据展开、provider 选择和图遍历均使用工作队列或显式栈。运行期只消费编译结果，
//! 不得再次调用物化 callback、选择候选或修改图。
//!
//! 阅读顺序：`compiler` 展示编译阶段；本文件定义阶段的最终输出；`diagnostics` 把
//! 冻结结果转成只读图数据。这里的节点是 Provider 声明，运行期一次 Transient 消费
//! 产生的实例/任务不等同于图节点。

mod compiler;
mod diagnostics;
mod names;

use std::{collections::HashMap, fmt};

use crate::{
    activation::{ClassConstructor, InputPreparer, InputSlot, LazyInputPreparer},
    registration::provider::{FactoryInvoker, ProviderCommon},
    service::{ServiceIdentifier, ServiceSource},
};

pub(crate) use compiler::GraphCompiler;
pub(crate) use diagnostics::snapshot;

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

/// 一份 Provider 声明的构造计划；生命周期决定运行期有多少个实际 occurrence。
#[derive(Debug)]
pub(crate) struct CompiledNode {
    pub(crate) identifier: ServiceIdentifier,
    pub(crate) common: ProviderCommon,
    /// 输入槽位完整保留，不能像拓扑边一样去重。
    pub(crate) dependencies: Vec<CompiledDependency>,
    pub(crate) constructor: Constructor,
    /// 本节点或其传递依赖包含 Scoped；据此拒绝 root 查询需要 Scope 的 Transient。
    pub(crate) requires_scope: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Constructor {
    Class(ClassConstructor),
    Factory(FactoryInvoker),
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

/// 内部聚合结构错误；公开 build 边界选择 panic，实际服务构造失败使用另一类错误。
#[derive(Debug)]
pub(crate) struct GraphError {
    pub(crate) diagnostics: Vec<GraphDiagnostic>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GraphDiagnosticKind {
    DuplicateProvider,
    DuplicateBinding,
    OrphanBinding,
    AmbiguousTrait,
    InvalidMetadata,
    MissingDependency,
    MaterializationMismatch,
    Cycle,
    ScopeRequired,
}

/// 诊断携带完整参与类型和来源，不依赖日志拼接反向恢复信息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphDiagnostic {
    pub(crate) kind: GraphDiagnosticKind,
    pub(crate) message: String,
    pub(crate) services: Vec<ServiceIdentifier>,
    pub(crate) sources: Vec<ServiceSource>,
}

impl GraphDiagnostic {
    fn new(kind: GraphDiagnosticKind, message: String) -> Self {
        Self {
            kind,
            message,
            services: Vec::new(),
            sources: Vec::new(),
        }
    }

    fn at(mut self, service: &ServiceIdentifier, source: ServiceSource) -> Self {
        self.services.push(service.clone());
        self.sources.push(source);
        self
    }
}

impl fmt::Display for GraphDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{:?}] {}", self.kind, self.message)?;
        for service in &self.services {
            if !self.message.contains(service.service_type.name) {
                write!(
                    formatter,
                    "；服务 {} [key={:?}]",
                    service.service_type.name, service.service_key
                )?;
            }
        }
        for source in &self.sources {
            let location = format!("{}:{}:{}", source.file, source.line, source.column);
            if !self.message.contains(&location) {
                write!(formatter, "；来源 {location}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "服务图验证失败（{} 项）", self.diagnostics.len())?;
        for diagnostic in &self.diagnostics {
            writeln!(formatter, "- {diagnostic}")?;
        }
        Ok(())
    }
}

impl std::error::Error for GraphError {}

#[cfg(test)]
#[path = "../../tests/unit/graph/compiler.rs"]
mod tests;
