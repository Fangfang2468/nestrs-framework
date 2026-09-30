//! 求注册声明的有限闭包，统一处理显式 Provider、闭合蓝图和按需 trait 投影。
//!
//! 蓝图与自动 binding 是能力目录，不能因为它们存在就注册所有可能服务。两个工作队列
//! 相互推进：依赖请求激活接口能力，接口能力又可能物化新的 concrete 声明。队列耗尽后
//! 才分配最终 ProviderId，避免回调发现顺序影响查询路由与诊断。

use std::collections::{HashMap, HashSet, VecDeque};

use super::{
    Constructor, Declaration, GraphDiagnostic, Kind, binding_order, sort_declarations, source,
    token,
};
use crate::{
    registration::{
        binding::TraitBinding,
        catalog::RegistrySnapshot,
        dependency::{ClosedProviderCallback, ProviderSource},
        provider::Provider,
        root::RootDeclaration,
    },
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};

/// 展开结束后的声明集合；此时仍是描述，还未选择 trait 候选或实例化服务。
pub(super) struct ExpandedDeclarations {
    pub(super) providers: Vec<Declaration>,
    pub(super) bindings: Vec<TraitBinding>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclarationOrigin {
    /// 来自编译器收集的实际注册；同 token 的多个显式声明永远是错误。
    Explicit,
    /// 由已知闭合类型的描述回调产生；重复触达在元数据一致时幂等。
    Blueprint,
}

/// catalog 中的位置只供本阶段寻址，来源独立记录，不依赖 Vec 下标区间表达语义。
struct CatalogEntry {
    index: usize,
    origin: DeclarationOrigin,
}

/// 只有这个对象能把新声明加入展开集合，重复与优先级规则集中在两个插入方法中。
struct DeclarationExpansion<'diagnostics> {
    providers: Vec<Declaration>,
    catalog: HashMap<ServiceIdentifier, CatalogEntry>,
    materialized: HashSet<(ServiceType, usize)>,
    diagnostics: &'diagnostics mut Vec<GraphDiagnostic>,
}

impl<'diagnostics> DeclarationExpansion<'diagnostics> {
    fn new(providers: Vec<Provider>, diagnostics: &'diagnostics mut Vec<GraphDiagnostic>) -> Self {
        let mut declarations: Vec<Declaration> = providers.into_iter().map(Into::into).collect();
        sort_declarations(&mut declarations);
        let mut expansion = Self {
            providers: Vec::with_capacity(declarations.len()),
            catalog: HashMap::new(),
            materialized: HashSet::new(),
            diagnostics,
        };
        for declaration in declarations {
            expansion.insert_explicit(declaration);
        }
        expansion
    }

    fn insert_explicit(&mut self, declaration: Declaration) {
        if let Some(existing) = self.catalog.get(&declaration.identifier) {
            let original = &self.providers[existing.index];
            self.diagnostics.push(
                GraphDiagnostic::new(
                    Kind::DuplicateProvider,
                    format!(
                        "重复 Provider {}：{} 与 {}（primary 不能覆盖 concrete 注册）",
                        token(&declaration.identifier),
                        source(original.common.source),
                        source(declaration.common.source),
                    ),
                )
                .at(&original.identifier, original.common.source)
                .at(&declaration.identifier, declaration.common.source),
            );
        } else {
            self.insert(declaration, DeclarationOrigin::Explicit);
        }
    }

    /// 仅精确 type/key 的显式声明阻止依赖 fallback；同类型的其他 key 不参与优先级。
    fn has_explicit(&self, token: &ServiceIdentifier) -> bool {
        self.catalog
            .get(token)
            .is_some_and(|entry| entry.origin == DeclarationOrigin::Explicit)
    }

    fn insert(&mut self, declaration: Declaration, origin: DeclarationOrigin) {
        self.catalog.insert(
            declaration.identifier.clone(),
            CatalogEntry {
                index: self.providers.len(),
                origin,
            },
        );
        self.providers.push(declaration);
    }

    /// 这里只执行“生成 Provider 描述”的回调，服务构造入口仍是未调用的函数指针。
    fn materialize(
        &mut self,
        expected: ServiceType,
        callback: ClosedProviderCallback,
        request_source: ServiceSource,
    ) {
        // 地址只用于避免同一回调重复调用，不用于认定两个声明语义相同。
        // 不同代码生成单元可能给等价回调不同地址，因此还要比较实际描述元数据。
        if !self.materialized.insert((expected, callback as usize)) {
            return;
        }
        let declaration = Declaration::from(callback());
        if declaration.identifier.service_type != expected {
            self.diagnostics.push(
                GraphDiagnostic::new(
                    Kind::MaterializationMismatch,
                    format!(
                        "闭合 Provider 回调返回类型不匹配：请求 {}，实际 {}（{}）",
                        expected.name,
                        declaration.identifier.service_type.name,
                        source(request_source),
                    ),
                )
                .at(&expected.into(), request_source)
                .at(&declaration.identifier, declaration.common.source),
            );
            return;
        }
        // 描述回调保留自身的 key；不能用请求 key 改写它来“补齐”未注册服务。
        if let Some(existing) = self.catalog.get(&declaration.identifier) {
            if existing.origin == DeclarationOrigin::Blueprint
                && !same_metadata(&self.providers[existing.index], &declaration)
            {
                self.diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::InvalidMetadata,
                        format!(
                            "闭合 Provider 回调声明冲突：{}；同一 token 的可观察元数据不一致",
                            token(&declaration.identifier),
                        ),
                    )
                    .at(
                        &self.providers[existing.index].identifier,
                        self.providers[existing.index].common.source,
                    )
                    .at(&declaration.identifier, declaration.common.source),
                );
            }
            return;
        }
        self.insert(declaration, DeclarationOrigin::Blueprint);
    }
}

pub(super) fn expand(
    snapshot: RegistrySnapshot,
    diagnostics: &mut Vec<GraphDiagnostic>,
) -> ExpandedDeclarations {
    let RegistrySnapshot {
        providers,
        mut bindings,
        mut roots,
        mut automatic_bindings,
        mut blueprints,
        // 启动选项只决定冻结之后如何激活，不参与图结构与路由选择。
        options: _,
    } = snapshot;

    blueprints.sort_by_key(|entry| (entry.service_type.name, entry.source));
    let mut blueprint_catalog: HashMap<ServiceType, Vec<ClosedProviderCallback>> = HashMap::new();
    for blueprint in blueprints {
        if let Some(callback) = blueprint.materialize {
            blueprint_catalog
                .entry(blueprint.service_type)
                .or_default()
                .push(callback);
        }
    }
    let mut expansion = DeclarationExpansion::new(providers, diagnostics);

    // 上游投影是被动能力。仅在完整入口真正请求该接口时启用，且不能覆盖显式 pair。
    // 多个 crate 可以贡献同一自动 pair；其幂等不能掩盖重复的显式 binding。
    automatic_bindings.sort_by_key(binding_order);
    let mut known_pairs: HashSet<_> = bindings
        .iter()
        .map(|binding| (binding.trait_type, binding.concrete_type))
        .collect();
    let mut automatic_by_interface: HashMap<ServiceType, Vec<TraitBinding>> = HashMap::new();
    for binding in automatic_bindings {
        if known_pairs.insert((binding.trait_type, binding.concrete_type)) {
            automatic_by_interface
                .entry(binding.trait_type)
                .or_default()
                .push(binding);
        }
    }
    let mut requested_interfaces: VecDeque<_> =
        roots.iter().map(|root| root.service_type).collect();
    let mut visited_interfaces = HashSet::new();

    // 显式 binding 的闭合 self 类型本身也是声明锚点；只查询 dyn Trait 时，也必须
    // 先发现对应泛型 Provider，再判断 binding 是否孤立。
    roots.extend(bindings.iter().filter_map(|binding| {
        binding.materialize.map(|callback| RootDeclaration {
            service_type: binding.concrete_type,
            materialize: Some(callback),
            source: binding.source,
        })
    }));
    roots.sort_by_key(|root| (root.service_type.name, root.source));
    for root in roots {
        for callback in root.materialize.into_iter().chain(
            blueprint_catalog
                .get(&root.service_type)
                .into_iter()
                .flatten()
                .copied(),
        ) {
            expansion.materialize(root.service_type, callback, root.source);
        }
    }

    // cursor 是声明工作队列，requested_interfaces 是接口工作队列。新声明只追加到
    // providers，已读位置永不回退；接口集合与回调缓存也各自去重，因此不递归展开。
    let mut cursor = 0;
    while cursor < expansion.providers.len() || !requested_interfaces.is_empty() {
        if let Some(interface) = requested_interfaces.pop_front() {
            if visited_interfaces.insert(interface)
                && let Some(automatic) = automatic_by_interface.remove(&interface)
            {
                for binding in automatic {
                    if let Some(callback) = binding.materialize {
                        expansion.materialize(binding.concrete_type, callback, binding.source);
                    }
                    bindings.push(binding);
                }
            }
            continue;
        }
        // 克隆的是静态描述，不是实例；这样可以在遍历当前输入时安全追加新声明。
        let requests = expansion.providers[cursor].dependencies.clone();
        let request_source = expansion.providers[cursor].common.source;
        for request in requests {
            requested_interfaces.push_back(request.token.service_type);
            if !expansion.has_explicit(&request.token) {
                let callback = match request.provider_source {
                    ProviderSource::Materialize(callback) => Some(callback),
                    ProviderSource::Registered => None,
                };
                for callback in callback.into_iter().chain(
                    blueprint_catalog
                        .get(&request.token.service_type)
                        .into_iter()
                        .flatten()
                        .copied(),
                ) {
                    expansion.materialize(request.token.service_type, callback, request_source);
                }
            }
        }
        cursor += 1;
    }

    // catalog 已完成使命。最后排序后才让下阶段使用 Vec 下标作为 ProviderId。
    sort_declarations(&mut expansion.providers);
    ExpandedDeclarations {
        providers: expansion.providers,
        bindings,
    }
}

/// 无法比较 adapter 函数体；typed ABI 负责类型安全，同 token 的可观察声明属性仍须一致。
fn same_metadata(left: &Declaration, right: &Declaration) -> bool {
    let constructor_kind = |constructor: Constructor| match constructor {
        Constructor::Class(_) => 0,
        Constructor::Factory(crate::registration::provider::FactoryInvoker::Sync(_)) => 1,
        Constructor::Factory(crate::registration::provider::FactoryInvoker::Async(_)) => 2,
    };
    left.identifier == right.identifier
        && left.common.lifetime == right.common.lifetime
        && left.common.primary == right.common.primary
        && left.common.source == right.common.source
        && left.common.cleanup.is_some() == right.common.cleanup.is_some()
        && constructor_kind(left.constructor) == constructor_kind(right.constructor)
        && left.dependencies.len() == right.dependencies.len()
        && left
            .dependencies
            .iter()
            .zip(&right.dependencies)
            .all(|(left, right)| {
                left.token == right.token
                    && left.input_slot == right.input_slot
                    && left.declaration_position == right.declaration_position
                    && left.optional == right.optional
                    && left.lazy.is_some() == right.lazy.is_some()
                    && left.label == right.label
                    && std::mem::discriminant(&left.delivery)
                        == std::mem::discriminant(&right.delivery)
                    && std::mem::discriminant(&left.provider_source)
                        == std::mem::discriminant(&right.provider_source)
            })
}
