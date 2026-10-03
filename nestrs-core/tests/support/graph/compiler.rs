//! 服务图编译流水线：展开声明、选择路由、编译输入、验证图，最后冻结执行计划。
//!
//! 各阶段共享诊断集合而不提前短路，因此未被查询的服务、optional 的已存在候选、
//! 泛型物化结果也接受完整检查。所有临时状态都在本次编译内；成功后只交付不可变图，
//! 运行期不再访问注册回调、候选目录或泛型蓝图。

#[path = "compiler/expand.rs"]
mod expand;
#[path = "compiler/inputs.rs"]
mod inputs;
#[path = "compiler/routes.rs"]
mod routes;
#[path = "compiler/topology.rs"]
mod topology;

use super::{
    AbsentInput, CompiledDependency, CompiledNode, Constructor, DependencyInput, GraphDiagnostic,
    GraphDiagnosticKind as Kind, GraphError, ProviderId, RootRoute, ValidatedGraph,
};
use crate::{
    registration::{
        binding::TraitBinding,
        catalog::RegistrySnapshot,
        dependency::DependencyRequest,
        provider::{Provider, ProviderCommon},
    },
    service::{ServiceIdentifier, ServiceSource},
};

pub(crate) struct GraphCompiler;

/// Class 与 Factory 共享图算法所需字段，但构造入口仍保留原有的两类语义。
/// 这是编译期间的声明视图，不是另一种 Provider，也不持有服务实例。
struct Declaration {
    identifier: ServiceIdentifier,
    common: ProviderCommon,
    dependencies: Vec<DependencyRequest>,
    constructor: Constructor,
}

impl From<Provider> for Declaration {
    fn from(provider: Provider) -> Self {
        match provider {
            Provider::Class(provider) => Self {
                identifier: provider.provide,
                common: provider.common,
                dependencies: provider.dependencies,
                constructor: Constructor::Class(provider.constructor),
            },
            Provider::Factory(provider) => Self {
                identifier: provider.provide,
                common: provider.common,
                dependencies: provider.dependencies,
                constructor: Constructor::Factory(provider.invoker),
            },
        }
    }
}

impl GraphCompiler {
    /// 隔离测试的参考编译入口；生产只装载工具链已经验证的不可变计划。
    pub(crate) fn compile_snapshot(
        snapshot: RegistrySnapshot,
    ) -> Result<ValidatedGraph, GraphError> {
        let mut diagnostics = Vec::new();

        // 1. 求声明闭包。只有实际根、依赖与显式 binding 会激活被动蓝图/自动投影。
        let expand::ExpandedDeclarations {
            mut providers,
            bindings,
        } = expand::expand(snapshot, &mut diagnostics);
        // 2. 冻结精确 type/key 的 concrete 路由和 trait 候选选择。
        let selection = routes::select(&providers, bindings, &mut diagnostics);
        // 3. 保留每个输入槽位，固定它的目标与 typed preparer（包括 optional 缺席）。
        let mut nodes = inputs::compile(&mut providers, &selection, &mut diagnostics);
        // 4. 非递归检查循环并传播 Scope 要求；不会触发任何用户构造。
        let topology = topology::analyze(&mut nodes, &mut diagnostics);

        // 各阶段都运行完后统一排序与归并，诊断不受注册收集顺序影响。
        if !diagnostics.is_empty() {
            diagnostics.sort_by(|left, right| left.message.cmp(&right.message));
            diagnostics.dedup();
            return Err(GraphError { diagnostics });
        }
        Ok(ValidatedGraph {
            nodes,
            routes: selection.roots,
            topological_order: topology.order,
            dependents: topology.dependents,
        })
    }
}

fn binding_order(binding: &TraitBinding) -> (&'static str, &'static str, ServiceSource) {
    (
        binding.trait_type.name,
        binding.concrete_type.name,
        binding.source,
    )
}

fn sort_declarations(declarations: &mut [Declaration]) {
    declarations.sort_by(|left, right| {
        compare_tokens(&left.identifier, &right.identifier)
            .then(left.common.source.cmp(&right.common.source))
    });
}

fn compare_tokens(left: &ServiceIdentifier, right: &ServiceIdentifier) -> std::cmp::Ordering {
    left.service_type
        .name
        .cmp(right.service_type.name)
        .then(left.service_key.cmp(&right.service_key))
        .then(left.service_type.type_id.cmp(&right.service_type.type_id))
}

fn dependency_label(request: &DependencyRequest) -> String {
    request
        .label
        .map(|label| format!("字段/参数 `{label}`"))
        .unwrap_or_else(|| format!("声明位置 {}", request.declaration_position))
}

fn token(identifier: &ServiceIdentifier) -> String {
    format!(
        "{} [key={:?}]",
        identifier.service_type.name, identifier.service_key
    )
}

fn source(source: ServiceSource) -> String {
    format!("{}:{}:{}", source.file, source.line, source.column)
}
