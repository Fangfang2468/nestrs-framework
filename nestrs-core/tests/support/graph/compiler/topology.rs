//! 非递归拓扑分析、实际环定位与 Scope 能力传播。
//!
//! 排序只使用去重的 Provider 边，原始输入槽位仍保存在 CompiledNode 中。依赖优先顺序
//! 同时供生命周期检查和运行期预热使用；环诊断使用显式栈，避免深层服务图消耗调用栈。

use super::{CompiledNode, GraphDiagnostic, Kind, ProviderId, source, token};
use crate::ServiceLifetime;
use std::collections::{BTreeSet, VecDeque};

/// 有环时仍报告实际环；只有 DAG 才能沿依赖优先顺序传播 Scope 要求。
pub(super) fn analyze(
    nodes: &mut [CompiledNode],
    diagnostics: &mut Vec<GraphDiagnostic>,
) -> Topology {
    let topology = topological_order(nodes);
    let topological_order = &topology.order;
    let adjacency = &topology.dependencies;
    if topological_order.len() != nodes.len() {
        diagnostics.extend(cycle_diagnostics(nodes, adjacency));
    } else {
        // 依赖优先保证访问 provider 时，所有直接依赖的 requires_scope 已经确定。
        // witness 只记录一条证据边，就能为 Singleton 输出经过 Transient 的完整路径；
        // 不需要为每个节点复制整个依赖闭包。
        let mut scope_witness = vec![None; nodes.len()];
        for &provider in topological_order {
            if let Some(target) = nodes[provider]
                .dependencies
                .iter()
                .filter_map(|dependency| dependency.input.target())
                .find(|&target| nodes[target].requires_scope)
            {
                nodes[provider].requires_scope = true;
                scope_witness[provider] = Some(target);
            }
        }
        for (provider, node) in nodes.iter().enumerate() {
            if node.common.lifetime == ServiceLifetime::Singleton && node.requires_scope {
                let mut path = vec![provider];
                let mut current = provider;
                while nodes[current].common.lifetime != ServiceLifetime::Scoped {
                    let Some(next) = scope_witness[current] else {
                        break;
                    };
                    path.push(next);
                    current = next;
                }
                let mut diagnostic = GraphDiagnostic::new(
                    Kind::ScopeRequired,
                    format!(
                        "Singleton 的激活依赖需要 Scope：{}",
                        describe_path(nodes, &path),
                    ),
                );
                for &provider in &path {
                    diagnostic =
                        diagnostic.at(&nodes[provider].identifier, nodes[provider].common.source);
                }
                diagnostics.push(diagnostic);
            }
        }
    }
    topology
}

pub(super) struct Topology {
    pub(super) order: Vec<ProviderId>,
    dependencies: Vec<Vec<ProviderId>>,
    pub(super) dependents: Vec<Vec<ProviderId>>,
}

/// Kahn 算法：剩余依赖数降到零即可入队，不递归沿服务链展开。
fn topological_order(nodes: &[CompiledNode]) -> Topology {
    // 一个依赖可能占多个输入槽位，但只能向拓扑计数贡献一条边。BTreeSet 同时保证
    // 邻接顺序稳定；真实槽位仍留在节点中，Transient 的独立消费不受影响。
    let adjacency: Vec<Vec<_>> = nodes
        .iter()
        .map(|node| {
            node.dependencies
                .iter()
                .filter_map(|dependency| dependency.input.target())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        })
        .collect();
    let mut consumers = vec![Vec::new(); nodes.len()];
    let mut remaining: Vec<_> = adjacency.iter().map(Vec::len).collect();
    for (provider, dependencies) in adjacency.iter().enumerate() {
        for &dependency in dependencies {
            consumers[dependency].push(provider);
        }
    }
    let mut ready: VecDeque<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(provider, &count)| (count == 0).then_some(provider))
        .collect();
    let mut order = Vec::with_capacity(nodes.len());
    while let Some(provider) = ready.pop_front() {
        order.push(provider);
        for &consumer in &consumers[provider] {
            remaining[consumer] -= 1;
            if remaining[consumer] == 0 {
                ready.push_back(consumer);
            }
        }
    }
    Topology {
        order,
        dependencies: adjacency,
        dependents: consumers,
    }
}

/// Kahn 残余集合也包含被环阻塞的下游节点，不能直接把所有残余节点称作一个环。
/// 这里用显式 DFS 帧定位回边，再从当前路径截取真正的环，保持深链下的固定调用栈。
fn cycle_diagnostics(
    nodes: &[CompiledNode],
    adjacency: &[Vec<ProviderId>],
) -> Vec<GraphDiagnostic> {
    // 0=未访问，1=仍在当前路径，2=已完成。positions 使回边能直接定位路径起点。
    let mut colors = vec![0_u8; nodes.len()];
    let mut positions = vec![0; nodes.len()];
    let mut diagnostics = Vec::new();
    for root in 0..nodes.len() {
        if colors[root] != 0 {
            continue;
        }
        colors[root] = 1;
        // 每帧保存节点及下一条待访问边，作用等价于递归函数的局部遍历状态。
        let mut stack = vec![(root, 0)];
        positions[root] = 0;
        while let Some((provider, next)) = stack.last_mut() {
            if *next == adjacency[*provider].len() {
                colors[*provider] = 2;
                stack.pop();
                continue;
            }
            let target = adjacency[*provider][*next];
            *next += 1;
            match colors[target] {
                0 => {
                    colors[target] = 1;
                    positions[target] = stack.len();
                    stack.push((target, 0));
                }
                1 => {
                    let mut cycle: Vec<_> = stack[positions[target]..]
                        .iter()
                        .map(|(provider, _)| *provider)
                        .collect();
                    cycle.push(target);
                    let mut diagnostic = GraphDiagnostic::new(
                        Kind::Cycle,
                        format!("循环依赖：{}", describe_path(nodes, &cycle)),
                    );
                    for &provider in &cycle {
                        diagnostic = diagnostic
                            .at(&nodes[provider].identifier, nodes[provider].common.source);
                    }
                    diagnostics.push(diagnostic);
                }
                _ => {}
            }
        }
    }
    diagnostics
}

/// 从原始输入槽位恢复边的字段/参数标签，诊断同时保留精确 key 与声明位置。
fn describe_path(nodes: &[CompiledNode], path: &[ProviderId]) -> String {
    path.iter()
        .enumerate()
        .map(|(index, &provider)| {
            let node = &nodes[provider];
            let edge = path.get(index + 1).and_then(|next| {
                node.dependencies
                    .iter()
                    .find(|dependency| dependency.input.target() == Some(*next))
            });
            let suffix = edge
                .map(|edge| match edge.label {
                    Some(label) => format!(".{label}"),
                    None => format!("[slot {}]", edge.slot.index()),
                })
                .unwrap_or_default();
            format!(
                "{}{}（{}）",
                token(&node.identifier),
                suffix,
                source(node.common.source)
            )
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}
