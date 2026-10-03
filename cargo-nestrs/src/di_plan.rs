//! 已闭合 DI 声明的纯语义计划编译。
//!
//! rustc 前端负责真实类型身份、泛型展开及 typed adapter 的合法性；本模块只消费已
//! 展开的声明。类型名字只用于诊断，所有比较使用前端分配的类型 ID。输出中的索引
//! 保持输入顺序，运行时无需再次选择 provider、投影或判断依赖图是否有效。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Type {
    pub id: usize,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Default,
    Named(String),
    Indexed(u128),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lifetime {
    Singleton,
    Scoped,
    Transient,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub type_id: usize,
    pub key: Key,
    pub lifetime: Lifetime,
    pub primary: bool,
    /// None 继承容器配置，Some(true) 延迟预热，Some(false) 显式提前初始化。
    /// 只决定预热根选择；全部声明与依赖仍接受完整图检查。
    pub lazy: Option<bool>,
    pub source: String,
    pub inputs: Vec<Input>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub type_id: usize,
    pub key: Key,
    pub slot: usize,
    pub optional: bool,
    pub lazy: bool,
    pub label: String,
}

/// 本层的 binding 均为实际启用的 pair。自动能力目录的按需启用与幂等合并属于前端。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub concrete: usize,
    pub interface: usize,
    pub source: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputPlan {
    pub target: Option<usize>,
    /// 选中 trait 投影在输入 binding 数组中的索引；concrete 和缺席输入为 None。
    pub binding: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub type_id: usize,
    pub key: Key,
    pub provider: usize,
    pub binding: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// 每个 provider 的全部输入槽位；重复 target 不能合并，否则会丢失 Transient 消费。
    pub inputs: Vec<Vec<InputPlan>>,
    pub requires_scope: Vec<bool>,
    /// 依赖优先，稳定地选择当前可处理的最小 provider 索引。
    pub order: Vec<usize>,
    /// 去重后的反向边；lazy 边同样保留以供生命周期验证和关闭顺序使用。
    pub dependents: Vec<Vec<usize>>,
    pub routes: Vec<Route>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagnosticKind {
    InvalidMetadata,
    DuplicateProvider,
    DuplicateBinding,
    OrphanBinding,
    AmbiguousTrait,
    MissingDependency,
    Cycle,
    ScopeRequired,
}

/// 错误涉及的真实输入边。索引对应本次 `compile` 的输入模型，不是跨阶段身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DependencyEdge {
    pub consumer: usize,
    /// `providers[consumer].inputs` 中的位置。有效模型中等于 `Input::slot`；
    /// 非法元数据仍用数组位置保留可访问的证据，不跟随错误的槽位值。
    pub slot: usize,
    pub target: usize,
}

/// 交给编译前端关联源码的错误事实，不从诊断文本反推类型、候选或依赖关系。
///
/// provider/binding 索引均来自本次编译输入；属性及输入修饰直接读取原始声明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticEvidence {
    InvalidMetadata,
    DuplicateProvider {
        first: usize,
        duplicate: usize,
    },
    DuplicateBinding {
        first: usize,
        duplicate: usize,
    },
    OrphanBinding {
        binding: usize,
    },
    AmbiguousTrait {
        type_id: usize,
        key: Key,
        candidates: Vec<usize>,
    },
    MissingDependency {
        consumer: usize,
        slot: usize,
    },
    Cycle {
        edges: Vec<DependencyEdge>,
    },
    ScopeRequired {
        singleton: usize,
        scoped: usize,
        edges: Vec<DependencyEdge>,
    },
}

impl DiagnosticEvidence {
    fn kind(&self) -> DiagnosticKind {
        match self {
            Self::InvalidMetadata => DiagnosticKind::InvalidMetadata,
            Self::DuplicateProvider { .. } => DiagnosticKind::DuplicateProvider,
            Self::DuplicateBinding { .. } => DiagnosticKind::DuplicateBinding,
            Self::OrphanBinding { .. } => DiagnosticKind::OrphanBinding,
            Self::AmbiguousTrait { .. } => DiagnosticKind::AmbiguousTrait,
            Self::MissingDependency { .. } => DiagnosticKind::MissingDependency,
            Self::Cycle { .. } => DiagnosticKind::Cycle,
            Self::ScopeRequired { .. } => DiagnosticKind::ScopeRequired,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// 内部完整说明，供最终诊断的 cause 使用；用户正文由 evidence 与真实源码组成。
    pub message: String,
    pub evidence: DiagnosticEvidence,
}

/// 编译已展开的全部声明；未被其他 provider 使用的声明也接受完整检查。
///
/// 不执行描述回调、构造器或投影。所有遍历均显式迭代，名字只用于输出诊断。前端需
/// 为同一真实类型分配同一 ID，并在调用前稳定排序 providers；binding 索引不被重排。
pub fn compile(
    types: &[Type],
    providers: &[Provider],
    bindings: &[Binding],
) -> Result<Plan, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    let mut names = BTreeMap::new();
    for ty in types {
        if names.insert(ty.id, ty.name.as_str()).is_some() {
            diagnostic(
                &mut diagnostics,
                DiagnosticEvidence::InvalidMetadata,
                format!("类型 ID {} 重复：{}", ty.id, ty.name),
            );
        }
    }
    let context = Context { names, providers };
    for (provider_id, provider) in providers.iter().enumerate() {
        context.check_type(provider.type_id, &provider.source, &mut diagnostics);
        for (position, input) in provider.inputs.iter().enumerate() {
            let location = context.input(provider_id, position);
            context.check_type(input.type_id, &location, &mut diagnostics);
            if input.slot != position {
                diagnostic(
                    &mut diagnostics,
                    DiagnosticEvidence::InvalidMetadata,
                    format!(
                        "输入槽位必须连续且唯一：{location}，声明槽位 {}，期望 {position}",
                        input.slot
                    ),
                );
            }
        }
    }

    // Some 是唯一选择；None 保留“此路由已被歧义诊断”的事实，避免再误报缺失。
    let mut routes: BTreeMap<(usize, Key), Option<Route>> = BTreeMap::new();
    let mut providers_by_type: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (id, provider) in providers.iter().enumerate() {
        providers_by_type
            .entry(provider.type_id)
            .or_default()
            .push(id);
        let key = (provider.type_id, provider.key.clone());
        if let Some(previous) = routes.get(&key) {
            let first = previous
                .as_ref()
                .expect("concrete 路由总是唯一候选")
                .provider;
            diagnostic(
                &mut diagnostics,
                DiagnosticEvidence::DuplicateProvider {
                    first,
                    duplicate: id,
                },
                format!(
                    "同一 concrete 类型与 key 存在重复 provider：{}；{}",
                    context.provider(first),
                    context.provider(id)
                ),
            );
        } else {
            routes.insert(
                key,
                Some(Route {
                    type_id: provider.type_id,
                    key: provider.key.clone(),
                    provider: id,
                    binding: None,
                }),
            );
        }
    }

    let mut pairs = BTreeMap::new();
    let mut candidates: BTreeMap<(usize, Key), Vec<(usize, usize)>> = BTreeMap::new();
    for (binding_id, binding) in bindings.iter().enumerate() {
        context.check_type(binding.concrete, &binding.source, &mut diagnostics);
        context.check_type(binding.interface, &binding.source, &mut diagnostics);
        if binding.concrete == binding.interface {
            diagnostic(
                &mut diagnostics,
                DiagnosticEvidence::InvalidMetadata,
                format!(
                    "binding 的 concrete 与 interface 类型相同：{}（{}）",
                    context.ty(binding.concrete),
                    binding.source
                ),
            );
        }
        if let Some(&previous) = pairs.get(&(binding.concrete, binding.interface)) {
            diagnostic(
                &mut diagnostics,
                DiagnosticEvidence::DuplicateBinding {
                    first: previous,
                    duplicate: binding_id,
                },
                format!(
                    "重复 binding：{} -> {}（{}；{}）",
                    context.ty(binding.concrete),
                    context.ty(binding.interface),
                    bindings[previous].source,
                    binding.source
                ),
            );
            continue;
        }
        pairs.insert((binding.concrete, binding.interface), binding_id);
        if let Some(matched) = providers_by_type.get(&binding.concrete) {
            for &provider_id in matched {
                candidates
                    .entry((binding.interface, providers[provider_id].key.clone()))
                    .or_default()
                    .push((provider_id, binding_id));
            }
        } else {
            diagnostic(
                &mut diagnostics,
                DiagnosticEvidence::OrphanBinding {
                    binding: binding_id,
                },
                format!(
                    "binding 找不到对应 concrete provider：{} -> {}（{}）",
                    context.ty(binding.concrete),
                    context.ty(binding.interface),
                    binding.source
                ),
            );
        }
    }
    for ((type_id, key), candidates) in candidates {
        if routes.contains_key(&(type_id, key.clone())) {
            diagnostic(
                &mut diagnostics,
                DiagnosticEvidence::InvalidMetadata,
                format!(
                    "类型同时作为 concrete provider 与 trait 路由：{} {}",
                    context.ty(type_id),
                    key_text(&key)
                ),
            );
            continue;
        }
        let selected = if candidates.len() == 1 {
            Some(candidates[0])
        } else {
            let primary: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|(provider, _)| providers[*provider].primary)
                .collect();
            if primary.len() == 1 {
                Some(primary[0])
            } else {
                let reason = if primary.is_empty() {
                    "没有唯一 primary"
                } else {
                    "存在多个 primary"
                };
                diagnostic(
                    &mut diagnostics,
                    DiagnosticEvidence::AmbiguousTrait {
                        type_id,
                        key: key.clone(),
                        candidates: candidates.iter().map(|(provider, _)| *provider).collect(),
                    },
                    format!(
                        "trait 候选歧义：{} {}，{reason}；候选：{}",
                        context.ty(type_id),
                        key_text(&key),
                        candidates
                            .iter()
                            .map(|(provider, _)| context.provider(*provider))
                            .collect::<Vec<_>>()
                            .join("；")
                    ),
                );
                None
            }
        };
        routes.insert(
            (type_id, key.clone()),
            selected.map(|(provider, binding)| Route {
                type_id,
                key,
                provider,
                binding: Some(binding),
            }),
        );
    }

    let mut inputs = Vec::with_capacity(providers.len());
    for (provider_id, provider) in providers.iter().enumerate() {
        let mut selected = Vec::with_capacity(provider.inputs.len());
        for (position, input) in provider.inputs.iter().enumerate() {
            let route = routes.get(&(input.type_id, input.key.clone()));
            if route.is_none() && !input.optional {
                diagnostic(
                    &mut diagnostics,
                    DiagnosticEvidence::MissingDependency {
                        consumer: provider_id,
                        slot: position,
                    },
                    format!("缺少必选依赖：{}", context.input(provider_id, position)),
                );
            }
            let route = route.and_then(Option::as_ref);
            selected.push(InputPlan {
                target: route.map(|route| route.provider),
                binding: route.and_then(|route| route.binding),
            });
        }
        inputs.push(selected);
    }

    let (order, dependents) = topology(&inputs);
    if order.len() != providers.len() {
        report_cycles(&context, &inputs, &dependents, &mut diagnostics);
    }
    let requires_scope = scope_requirements(&context, &inputs, &dependents, &mut diagnostics);
    diagnostics
        .sort_by(|left, right| (left.kind, &left.message).cmp(&(right.kind, &right.message)));
    diagnostics.dedup();
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(Plan {
        inputs,
        requires_scope,
        order,
        dependents,
        routes: routes.into_values().flatten().collect(),
    })
}

/// Kahn 仅合并拓扑重复边，输入交付数组保持完整。
fn topology(inputs: &[Vec<InputPlan>]) -> (Vec<usize>, Vec<Vec<usize>>) {
    let mut remaining = Vec::with_capacity(inputs.len());
    let mut dependents = vec![Vec::new(); inputs.len()];
    for (consumer, slots) in inputs.iter().enumerate() {
        let unique: BTreeSet<_> = slots.iter().filter_map(|input| input.target).collect();
        remaining.push(unique.len());
        for dependency in unique {
            dependents[dependency].push(consumer);
        }
    }
    let mut ready: BTreeSet<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(id, &count)| (count == 0).then_some(id))
        .collect();
    let mut order = Vec::with_capacity(inputs.len());
    while let Some(provider) = ready.pop_first() {
        order.push(provider);
        for &consumer in &dependents[provider] {
            remaining[consumer] -= 1;
            if remaining[consumer] == 0 {
                ready.insert(consumer);
            }
        }
    }
    (order, dependents)
}

/// 迭代 Kosaraju 标记强连通分量，让同一根因只报告一个代表闭环。
fn components(inputs: &[Vec<InputPlan>], dependents: &[Vec<usize>]) -> Vec<usize> {
    let mut seen = vec![false; inputs.len()];
    let mut finished = Vec::with_capacity(inputs.len());
    for start in 0..inputs.len() {
        if seen[start] {
            continue;
        }
        let mut stack = vec![(start, 0)];
        seen[start] = true;
        while let Some(&(node, next)) = stack.last() {
            if next == inputs[node].len() {
                finished.push(node);
                stack.pop();
                continue;
            }
            stack.last_mut().expect("当前栈帧存在").1 += 1;
            if let Some(target) = inputs[node][next].target
                && !seen[target]
            {
                seen[target] = true;
                stack.push((target, 0));
            }
        }
    }
    let mut components = vec![usize::MAX; inputs.len()];
    for start in finished.into_iter().rev() {
        if components[start] != usize::MAX {
            continue;
        }
        let mut stack = vec![start];
        components[start] = start;
        while let Some(node) = stack.pop() {
            for &target in &dependents[node] {
                if components[target] == usize::MAX {
                    components[target] = start;
                    stack.push(target);
                }
            }
        }
    }
    components
}

/// 显式 DFS 栈记录真实输入边。每个有环分量只发出首条回边形成的闭环，
/// 不把拓扑排序残留的下游节点当成环，也不合并 optional/lazy 的交付槽位。
fn report_cycles(
    context: &Context<'_>,
    inputs: &[Vec<InputPlan>],
    dependents: &[Vec<usize>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    let component = components(inputs, dependents);
    let mut reported = vec![false; inputs.len()];
    let mut state = vec![0_u8; inputs.len()];
    let mut positions = vec![0; inputs.len()];
    for start in 0..inputs.len() {
        if state[start] != 0 {
            continue;
        }
        let mut stack = vec![(start, 0_usize, None)];
        state[start] = 1;
        while let Some(&(node, next, _)) = stack.last() {
            if next == inputs[node].len() {
                state[node] = 2;
                stack.pop();
                continue;
            }
            stack.last_mut().expect("当前栈帧存在").1 += 1;
            let Some(target) = inputs[node][next].target else {
                continue;
            };
            match state[target] {
                0 => {
                    positions[target] = stack.len();
                    state[target] = 1;
                    stack.push((target, 0, Some(next)));
                }
                1 if !reported[component[target]] => {
                    reported[component[target]] = true;
                    let mut edges = Vec::new();
                    for index in positions[target]..stack.len() - 1 {
                        edges.push(DependencyEdge {
                            consumer: stack[index].0,
                            slot: stack[index + 1].2.expect("非根帧有输入槽位"),
                            target: stack[index + 1].0,
                        });
                    }
                    edges.push(DependencyEdge {
                        consumer: node,
                        slot: next,
                        target,
                    });
                    let path = edges
                        .iter()
                        .map(|edge| context.input(edge.consumer, edge.slot))
                        .collect::<Vec<_>>();
                    diagnostic(
                        diagnostics,
                        DiagnosticEvidence::Cycle { edges },
                        format!(
                            "循环依赖：{} -> {}",
                            path.join(" -> "),
                            context.provider(target)
                        ),
                    );
                }
                _ => {}
            }
        }
    }
}

/// 从所有 Scoped 反向传播能力，包含 optional 已选边与 lazy 边，且不依赖图无环。
fn scope_requirements(
    context: &Context<'_>,
    inputs: &[Vec<InputPlan>],
    dependents: &[Vec<usize>],
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<bool> {
    let mut needed: Vec<_> = context
        .providers
        .iter()
        .map(|provider| provider.lifetime == Lifetime::Scoped)
        .collect();
    let mut queue: VecDeque<_> = needed
        .iter()
        .enumerate()
        .filter_map(|(id, &needs_scope)| needs_scope.then_some(id))
        .collect();
    let mut witness = vec![None; inputs.len()];
    while let Some(dependency) = queue.pop_front() {
        for &consumer in &dependents[dependency] {
            if !needed[consumer] {
                needed[consumer] = true;
                let slot = inputs[consumer]
                    .iter()
                    .position(|input| input.target == Some(dependency))
                    .expect("反向边对应一个输入");
                witness[consumer] = Some((dependency, slot));
                queue.push_back(consumer);
            }
        }
    }
    for (provider, definition) in context.providers.iter().enumerate() {
        if definition.lifetime != Lifetime::Singleton || !needed[provider] {
            continue;
        }
        let mut path = Vec::new();
        let mut edges = Vec::new();
        let mut current = provider;
        while let Some((dependency, slot)) = witness[current] {
            path.push(context.input(current, slot));
            edges.push(DependencyEdge {
                consumer: current,
                slot,
                target: dependency,
            });
            current = dependency;
        }
        path.push(context.provider(current));
        diagnostic(
            diagnostics,
            DiagnosticEvidence::ScopeRequired {
                singleton: provider,
                scoped: current,
                edges,
            },
            format!("Singleton 的激活依赖需要 Scope：{}", path.join(" -> ")),
        );
    }
    needed
}

fn diagnostic(output: &mut Vec<Diagnostic>, evidence: DiagnosticEvidence, message: String) {
    output.push(Diagnostic {
        kind: evidence.kind(),
        message,
        evidence,
    });
}

fn key_text(key: &Key) -> String {
    match key {
        Key::Default => "[key=None]".into(),
        Key::Named(name) => format!("[key={name:?}]"),
        Key::Indexed(index) => format!("[key={index}]"),
    }
}

struct Context<'a> {
    names: BTreeMap<usize, &'a str>,
    providers: &'a [Provider],
}

impl Context<'_> {
    fn ty(&self, id: usize) -> String {
        self.names
            .get(&id)
            .map_or_else(|| format!("<未知类型#{id}>"), |name| (*name).to_owned())
    }
    fn provider(&self, id: usize) -> String {
        let provider = &self.providers[id];
        format!(
            "{} {}（{}）",
            self.ty(provider.type_id),
            key_text(&provider.key),
            provider.source
        )
    }
    fn input(&self, provider: usize, slot: usize) -> String {
        let input = &self.providers[provider].inputs[slot];
        format!(
            "{}.{}（槽位 {}）请求 {} {}{}",
            self.provider(provider),
            input.label,
            input.slot,
            self.ty(input.type_id),
            key_text(&input.key),
            if input.lazy { " [lazy]" } else { "" }
        )
    }
    fn check_type(&self, id: usize, source: &str, output: &mut Vec<Diagnostic>) {
        if !self.names.contains_key(&id) {
            diagnostic(
                output,
                DiagnosticEvidence::InvalidMetadata,
                format!("未知类型 ID {id}（{source}）"),
            );
        }
    }
}
