//! 在执行任何用户构造代码之前编译完整、不可变的服务图。
//!
//! 元数据展开、provider 选择和图遍历均使用工作队列或显式栈。运行期只消费编译结果，
//! 不得再次调用物化 callback、选择候选或修改图。

mod compiler;
mod diagnostics;
mod names;

use std::{collections::HashMap, fmt};

use crate::{
    activation::{ClassConstructor, InputPreparer, InputSlot},
    registration::provider::{FactoryInvoker, ProviderCommon},
    service::{ServiceIdentifier, ServiceSource},
};

pub(crate) use compiler::GraphCompiler;
pub(crate) use diagnostics::snapshot;

pub(crate) type ProviderId = usize;

#[derive(Debug)]
pub(crate) struct ValidatedGraph {
    pub(crate) nodes: Vec<CompiledNode>,
    pub(crate) routes: HashMap<ServiceIdentifier, RootRoute>,
    /// Dependencies appear before their consumers.
    pub(crate) topological_order: Vec<ProviderId>,
    /// Unique consumers of each provider, in deterministic ProviderId order.
    pub(crate) dependents: Vec<Vec<ProviderId>>,
}

#[derive(Debug)]
pub(crate) struct CompiledNode {
    pub(crate) identifier: ServiceIdentifier,
    pub(crate) common: ProviderCommon,
    pub(crate) dependencies: Vec<CompiledDependency>,
    pub(crate) constructor: Constructor,
    pub(crate) requires_scope: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Constructor {
    Class(ClassConstructor),
    Factory(FactoryInvoker),
}

#[derive(Debug, Clone)]
pub(crate) struct CompiledDependency {
    pub(crate) slot: InputSlot,
    /// Preserve the declared request for diagnostics, including trait projection and absence.
    pub(crate) requested: ServiceIdentifier,
    pub(crate) optional: bool,
    pub(crate) target: Option<ProviderId>,
    pub(crate) prepare: InputPreparer,
    pub(crate) label: Option<&'static str>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RootRoute {
    pub(crate) provider: ProviderId,
    /// Trait roots reuse the binding's typed required-input projection.
    pub(crate) projection: Option<InputPreparer>,
}

/// Structured aggregation remains private; the public build boundary chooses to panic.
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
mod tests;
