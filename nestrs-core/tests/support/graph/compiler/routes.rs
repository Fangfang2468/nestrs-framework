//! 选择 concrete 与 trait 的查询路由，固定每个精确 type/key 对应的 Provider。
//!
//! 展开阶段已经收集完整声明；本阶段不再物化类型，也不执行构造。
//! binding 只提供投影能力，同一个 concrete Provider 不会因多个接口而复制实例。

use std::collections::HashMap;

use super::{
    Declaration, GraphDiagnostic, Kind, ProviderId, RootRoute, binding_order, compare_tokens,
    source, token,
};
use crate::{
    registration::binding::TraitBinding,
    service::{ServiceIdentifier, ServiceType},
};

/// 查询路由供冻结图使用，binding 索引只在输入编译期间选择正确的 typed preparer。
pub(super) struct SelectedRoutes {
    pub(super) roots: HashMap<ServiceIdentifier, RootRoute>,
    pub(super) bindings: HashMap<(ServiceType, ServiceType), TraitBinding>,
}

/// 先建立 concrete 路由，再按 trait 与精确 key 分组；只有同组中的 primary 才参与决胜。
pub(super) fn select(
    providers: &[Declaration],
    mut bindings: Vec<TraitBinding>,
    diagnostics: &mut Vec<GraphDiagnostic>,
) -> SelectedRoutes {
    let mut roots: HashMap<_, _> = providers
        .iter()
        .enumerate()
        .map(|(provider, declaration)| {
            (
                declaration.identifier.clone(),
                RootRoute {
                    provider,
                    projection: None,
                },
            )
        })
        .collect();
    let mut by_type: HashMap<ServiceType, Vec<ProviderId>> = HashMap::new();
    for (provider, declaration) in providers.iter().enumerate() {
        by_type
            .entry(declaration.identifier.service_type)
            .or_default()
            .push(provider);
    }

    bindings.sort_by_key(binding_order);
    let mut binding_index = HashMap::new();
    let mut candidates: HashMap<ServiceIdentifier, Vec<(ProviderId, TraitBinding)>> =
        HashMap::new();
    for binding in bindings {
        if let Some(original) =
            binding_index.insert((binding.trait_type, binding.concrete_type), binding)
        {
            diagnostics.push(
                GraphDiagnostic::new(
                    Kind::DuplicateBinding,
                    format!(
                        "重复 trait binding {} -> {}：{} 与 {}",
                        binding.trait_type.name,
                        binding.concrete_type.name,
                        source(original.source),
                        source(binding.source),
                    ),
                )
                .at(&binding.trait_type.into(), original.source)
                .at(&binding.concrete_type.into(), binding.source),
            );
            continue;
        }
        let Some(provider_ids) = by_type.get(&binding.concrete_type) else {
            diagnostics.push(
                GraphDiagnostic::new(
                    Kind::OrphanBinding,
                    format!(
                        "孤立 trait binding {} -> {}：concrete 类型没有 Provider（{}）",
                        binding.trait_type.name,
                        binding.concrete_type.name,
                        source(binding.source),
                    ),
                )
                .at(&binding.trait_type.into(), binding.source)
                .at(&binding.concrete_type.into(), binding.source),
            );
            continue;
        };
        // key 跟随 concrete 注册。同类型的 named/indexed/default Provider 分别产生
        // 精确路由，binding 不引入另一套 key 配置，也不做默认 key 回退。
        for &provider in provider_ids {
            let identifier = ServiceIdentifier::new(
                providers[provider].identifier.service_key.clone(),
                binding.trait_type,
            );
            candidates
                .entry(identifier)
                .or_default()
                .push((provider, binding));
        }
    }

    // HashMap 只承担索引；在产生诊断与路由选择前显式排序，避免哈希/收集顺序影响结果。
    let mut candidate_groups: Vec<_> = candidates.into_iter().collect();
    candidate_groups.sort_by(|(left, _), (right, _)| compare_tokens(left, right));
    for (identifier, candidates) in candidate_groups {
        let primaries: Vec<_> = candidates
            .iter()
            .filter(|(provider, _)| providers[*provider].common.primary)
            .collect();
        let chosen = if candidates.len() == 1 {
            Some(&candidates[0])
        } else if primaries.len() == 1 {
            Some(primaries[0])
        } else {
            None
        };
        if let Some(&(provider, binding)) = chosen {
            if roots
                .insert(
                    identifier.clone(),
                    RootRoute {
                        provider,
                        projection: Some(binding.project),
                    },
                )
                .is_some()
            {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::InvalidMetadata,
                        format!("trait 路由与 concrete 注册冲突：{}", token(&identifier)),
                    )
                    .at(&identifier, binding.source),
                );
            }
        } else {
            let mut diagnostic = GraphDiagnostic::new(
                Kind::AmbiguousTrait,
                format!(
                    "trait 候选不唯一 {}（{} 个 primary）：{}",
                    token(&identifier),
                    primaries.len(),
                    candidates
                        .iter()
                        .map(|(provider, _)| {
                            format!(
                                "{}（{}）",
                                token(&providers[*provider].identifier),
                                source(providers[*provider].common.source)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("；"),
                ),
            );
            diagnostic.services.push(identifier);
            for (provider, binding) in &candidates {
                diagnostic = diagnostic
                    .at(&providers[*provider].identifier, binding.source)
                    .at(
                        &providers[*provider].identifier,
                        providers[*provider].common.source,
                    );
            }
            diagnostics.push(diagnostic);
        }
    }

    SelectedRoutes {
        roots,
        bindings: binding_index,
    }
}
