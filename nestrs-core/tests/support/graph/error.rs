//! 旧图算法的测试诊断模型，仅供迁移一致性与隔离声明测试使用。

use std::fmt;

use crate::service::{ServiceIdentifier, ServiceSource};

/// 隔离参考算法聚合的结构错误；生产中的对应错误由 cargo nestrs 编译器报告。
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
    pub(super) fn new(kind: GraphDiagnosticKind, message: String) -> Self {
        Self {
            kind,
            message,
            services: Vec::new(),
            sources: Vec::new(),
        }
    }

    pub(super) fn at(mut self, service: &ServiceIdentifier, source: ServiceSource) -> Self {
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
