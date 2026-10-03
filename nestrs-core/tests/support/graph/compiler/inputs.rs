//! 把声明期的依赖请求编译为运行期可直接使用的构造输入槽位。
//!
//! 此处完成 optional 缺席、concrete 输入与 trait 投影的最终选择。运行期无需再判断
//! 候选或泛型来源，只按冻结的 target/prepare 准备输入。即使多个输入指向相同 Provider，
//! 也保留全部槽位；对 Transient 而言，每个槽位代表独立的实例消费。

use super::{
    CompiledDependency, CompiledNode, Declaration, GraphDiagnostic, Kind, dependency_label,
    routes::SelectedRoutes, source, token,
};
use crate::{
    ServiceLifetime,
    activation::LazyInputPlan,
    registration::dependency::{Delivery, ProviderSource},
};
use std::sync::Arc;

/// 全部错误写入同一诊断集合；本阶段不会提前返回而隐藏其他 provider 的问题。
pub(super) fn compile(
    declarations: &mut [Declaration],
    selection: &SelectedRoutes,
    diagnostics: &mut Vec<GraphDiagnostic>,
) -> Vec<CompiledNode> {
    let provider_types: Vec<_> = declarations
        .iter()
        .map(|declaration| declaration.identifier.service_type)
        .collect();
    let mut nodes = Vec::with_capacity(declarations.len());
    for declaration in declarations.iter_mut() {
        // 声明位置可能包含 value/default 字段，构造槽位则必须从零开始连续且唯一。
        // 因此只按 input_slot 校验 ABI，不能拿 declaration_position 当数组下标。
        declaration
            .dependencies
            .sort_by_key(|dependency| dependency.input_slot);
        let mut dependencies = Vec::with_capacity(declaration.dependencies.len());
        for (position, request) in declaration.dependencies.iter().enumerate() {
            let location = format!(
                "{} 的 {}（{}）",
                token(&declaration.identifier),
                dependency_label(request),
                source(declaration.common.source)
            );
            if request.input_slot.index() != position {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::InvalidMetadata,
                        format!(
                            "输入槽位必须连续且唯一：{location} 声明 {:?}，期望 {position}",
                            request.input_slot,
                        ),
                    )
                    .at(&declaration.identifier, declaration.common.source),
                );
            }
            if matches!(
                (request.optional, request.delivery),
                (true, Delivery::RequiresBinding) | (false, Delivery::RequiresBindingOrAbsent(_))
            ) {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::InvalidMetadata,
                        format!("optional 与输入交付形态不一致：{location}"),
                    )
                    .at(&declaration.identifier, declaration.common.source),
                );
            }
            if !matches!(
                request.delivery,
                Delivery::Direct(_) | Delivery::Selected(_)
            ) && matches!(request.provider_source, ProviderSource::Materialize(_))
            {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::InvalidMetadata,
                        format!("trait 请求不能携带 concrete 物化回调：{location}"),
                    )
                    .at(&declaration.identifier, declaration.common.source),
                );
            }

            // 路由阶段已验证所有候选。optional 只允许路由不存在，不会隐藏歧义、环
            // 或生命周期问题；这些错误继续留在共享诊断集合中。
            let route = selection.roots.get(&request.token);
            if route.is_none() && !request.optional {
                diagnostics.push(
                    GraphDiagnostic::new(
                        Kind::MissingDependency,
                        format!("缺少必选依赖 {}：{location}", token(&request.token),),
                    )
                    .at(&declaration.identifier, declaration.common.source)
                    .at(&request.token, declaration.common.source),
                );
            }
            // Direct 是内部显式 concrete 协议，必须拒绝接口路由；Selected 则允许
            // 类型别名或 ?Sized 路径在完成类型分析后选择 concrete/trait 的真实交付。
            let prepare = match request.delivery {
                Delivery::Selected(prepare)
                    if route.is_none_or(|route| route.projection.is_none()) =>
                {
                    Some(prepare)
                }
                Delivery::Direct(prepare) => {
                    if route.is_some_and(|route| route.projection.is_some()) {
                        diagnostics.push(
                            GraphDiagnostic::new(
                                Kind::InvalidMetadata,
                                format!("concrete 输入不能使用 trait 路由：{location}"),
                            )
                            .at(&declaration.identifier, declaration.common.source),
                        );
                    }
                    Some(prepare)
                }
                Delivery::Selected(_)
                | Delivery::RequiresBinding
                | Delivery::RequiresBindingOrAbsent(_) => {
                    if let Some(route) = route {
                        let concrete_type = provider_types[route.provider];
                        let binding = selection
                            .bindings
                            .get(&(request.token.service_type, concrete_type));
                        match binding {
                            Some(binding) if route.projection.is_some() => {
                                Some(if request.optional {
                                    binding.prepare_optional
                                } else {
                                    binding.prepare_required
                                })
                            }
                            _ => {
                                diagnostics.push(
                                    GraphDiagnostic::new(
                                        Kind::InvalidMetadata,
                                        format!("trait 输入缺少类型化投影：{location}"),
                                    )
                                    .at(&declaration.identifier, declaration.common.source),
                                );
                                None
                            }
                        }
                    } else if let Delivery::RequiresBindingOrAbsent(absent) = request.delivery {
                        Some(absent)
                    } else {
                        None
                    }
                }
            };
            let lazy_plan = if request.lazy.is_some() {
                route.and_then(|route| {
                    let project = if route.projection.is_some() {
                        selection
                            .bindings
                            .get(&(request.token.service_type, provider_types[route.provider]))
                            .map(|binding| binding.project)
                    } else {
                        request.project
                    };
                    match project {
                        Some(project) => Some(Arc::new(LazyInputPlan {
                            provider: route.provider,
                            consumer: declaration.identifier.clone(),
                            source: declaration.common.source,
                            label: request.label,
                            input: request.input_slot,
                            project,
                        })),
                        None => {
                            diagnostics.push(
                                GraphDiagnostic::new(
                                    Kind::InvalidMetadata,
                                    format!("延迟输入缺少直接类型化投影：{location}"),
                                )
                                .at(&declaration.identifier, declaration.common.source),
                            );
                            None
                        }
                    }
                })
            } else {
                None
            };
            if let Some(prepare) = prepare {
                dependencies.push(CompiledDependency {
                    slot: request.input_slot,
                    requested: request.token.clone(),
                    optional: request.optional,
                    lazy: request.lazy,
                    lazy_plan,
                    target: route.map(|route| route.provider),
                    prepare,
                    label: request.label,
                });
            }
        }
        nodes.push(CompiledNode {
            identifier: declaration.identifier.clone(),
            common: declaration.common.into(),
            dependencies,
            constructor: declaration.constructor,
            requires_scope: declaration.common.lifetime == ServiceLifetime::Scoped,
        });
    }

    nodes
}
