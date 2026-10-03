//! 把图算法的错误证据关联回用户声明；所有索引都属于当前裁剪前的模型。
//! 来源独立于 core 执行协议，名称只用于展示，不参与候选选择。

use super::*;
use crate::diagnostics::{Diagnostic, source_span};
use model::{DependencyEdge, DiagnosticEvidence as Evidence};

/// 将元数据槽位映射回业务声明的诊断来源集合。
pub(super) struct Origin {
    /// 声明主位置，也是更细位置缺失时的回退值。
    pub declaration: Span,

    /// 完整服务类型的源码范围。
    pub service: Span,

    /// 声明名称，只用于诊断展示。
    pub name: String,

    /// 显式构造方法名称；自动字段构造时可为空。
    pub constructor: Option<String>,

    /// 输入槽位到请求类型源码范围的映射。
    pub inputs: HashMap<usize, Span>,

    /// 显式输入 key 的位置。
    pub input_keys: HashMap<usize, Span>,

    /// 尚待合并的输入类型末端 token 位置。
    input_ends: HashMap<usize, Span>,

    /// 尚待合并的服务类型末端 token 位置。
    service_end: Option<Span>,

    /// 生命周期配置的位置。
    pub lifetime: Option<Span>,

    /// primary 配置的位置。
    pub primary: Option<Span>,

    /// provider key 配置的位置。
    pub key: Option<Span>,
}

impl Origin {
    /// 以声明位置初始化来源集合，细粒度 marker 后续逐项覆盖。
    pub fn new(fallback: Span) -> Self {
        Self {
            declaration: fallback,
            service: fallback,
            name: String::new(),
            constructor: None,
            inputs: HashMap::new(),
            input_keys: HashMap::new(),
            input_ends: HashMap::new(),
            service_end: None,
            lifetime: None,
            primary: None,
            key: None,
        }
    }

    /// 只合并同一文件与卫生上下文中兼容的类型首尾范围。
    pub fn finish(&mut self, tcx: TyCtxt<'_>) {
        // proc_macro 的 join 不一定跨 token 可用；两端来自同一原始类型，
        // 但宏实参可能来自不同文件/上下文，因此只在原始区间兼容时合并。
        let join = |first: Span, last: Span| {
            if first.is_dummy()
                || last.is_dummy()
                || first.ctxt() != last.ctxt()
                || first.lo() > last.lo()
                || first.hi() > last.hi()
            {
                return first;
            }
            let map = tcx.sess.source_map();
            let file = map.lookup_source_file(first.lo());
            if last.hi() > file.end_position() {
                return first;
            }
            first.with_hi(last.hi())
        };
        for (slot, end) in &self.input_ends {
            if let Some(start) = self.inputs.get_mut(slot) {
                *start = join(*start, *end);
            }
        }
        if let Some(end) = self.service_end {
            self.service = join(self.service, end);
        }
    }

    /// 取得输入类型位置，缺失细粒度信息时回退到声明位置。
    pub fn input(&self, slot: usize) -> Span {
        self.inputs.get(&slot).copied().unwrap_or(self.declaration)
    }

    /// 解码一个来源 marker，按槽位保留配置或类型端点；未知协议种类返回错误。
    pub fn record(
        &mut self,
        kind: u128,
        slot: usize,
        span: Span,
        name: String,
    ) -> Result<(), String> {
        use crate::protocol::OriginKind;
        match kind {
            x if x == OriginKind::Declaration as u128 => {
                self.declaration = span;
                self.name = name;
            }
            x if x == OriginKind::Constructor as u128 => self.constructor = Some(name),
            x if x == OriginKind::Lifetime as u128 => self.lifetime = Some(span),
            x if x == OriginKind::Primary as u128 => self.primary = Some(span),
            x if x == OriginKind::ProviderKey as u128 => self.key = Some(span),
            x if x == OriginKind::InputKey as u128 => {
                self.input_keys.insert(slot, span);
            }
            x if x == OriginKind::InputTypeEnd as u128 => {
                self.input_ends.insert(slot, span);
            }
            x if x == OriginKind::ProviderTypeEnd as u128 => self.service_end = Some(span),
            _ => return Err(format!("未知诊断来源种类 {kind}")),
        }
        Ok(())
    }
}

/// 一次图错误渲染共享的类型名称、声明与查询来源。
struct Context<'a, 'tcx> {
    /// 用于解析源码位置和类型名称的当前会话。
    tcx: TyCtxt<'tcx>,

    /// 诊断证据中的类型索引目录。
    types: &'a [Ty<'tcx>],

    /// 必要时消除短名称冲突后的展示名称。
    names: Vec<String>,

    /// 裁剪前的 provider 目录。
    providers: &'a [Provider<'tcx>],

    /// 裁剪前的绑定目录。
    bindings: &'a [Binding<'tcx>],

    /// 查询根类型及引入该需求的位置。
    requests: &'a HashMap<Ty<'tcx>, Vec<Span>>,
}

/// 将纯图错误证据映射回业务来源，经统一 rustc 诊断出口终止当前编译。
pub(super) fn report<'tcx>(
    tcx: TyCtxt<'tcx>,
    types: &[Ty<'tcx>],
    providers: &[Provider<'tcx>],
    bindings: &[Binding<'tcx>],
    requests: &HashMap<Ty<'tcx>, Vec<Span>>,
    issues: &[model::Diagnostic],
) -> ! {
    let short: Vec<_> = types
        .iter()
        .map(|ty| rustc_middle::ty::print::with_forced_trimmed_paths!(ty.to_string()))
        .collect();
    let mut counts = HashMap::new();
    for label in &short {
        *counts.entry(label).or_insert(0) += 1;
    }
    let names = short
        .iter()
        .zip(types)
        .map(|(label, ty)| {
            if counts[label] > 1 {
                name(tcx, *ty)
            } else {
                label.clone()
            }
        })
        .collect();
    let context = Context {
        tcx,
        types,
        names,
        providers,
        bindings,
        requests,
    };
    crate::diagnostics::emit(
        tcx,
        issues.iter().map(|issue| context.render(issue)).collect(),
    )
}

/// 生成诊断中的 key 标签，区分默认、字符串与整数身份。
fn key_description(key: &model::Key) -> String {
    match key {
        model::Key::Default => "默认 key".into(),
        model::Key::Named(key) => format!("字符串 key={key:?}"),
        model::Key::Indexed(key) => format!("整数 key={key}"),
    }
}

impl<'tcx> Context<'_, 'tcx> {
    /// 根据 provider 的类型索引取得已消除冲突的展示名称。
    fn service(&self, provider: usize) -> &str {
        &self.names[self.providers[provider].data.type_id]
    }

    /// 优先展示工厂声明名称，其余 provider 展示服务类型名称。
    fn declaration(&self, provider: usize) -> String {
        let item = &self.providers[provider];
        if item.kind.contains("factory") && !item.origin.name.is_empty() {
            format!("工厂 `{}`", item.origin.name)
        } else {
            format!("服务 `{}`", self.service(provider))
        }
    }

    /// 按自动字段、显式构造或工厂形态组合输入的业务标签。
    fn input_name(&self, provider: usize, slot: usize) -> String {
        let item = &self.providers[provider];
        let label = &item.data.inputs[slot].label;
        if item.kind.contains("factory") {
            format!("工厂 `{}` 的参数 `{label}`", item.origin.name)
        } else if let Some(constructor) = &item.origin.constructor {
            format!(
                "`{}::{constructor}` 的参数 `{label}`",
                self.service(provider)
            )
        } else {
            format!("`{}.{label}`", self.service(provider))
        }
    }

    /// 定位所有请求同一 type/key 的消费者输入，供缺失或歧义诊断列出使用点。
    fn uses(&self, type_id: usize, key: &model::Key) -> Vec<(usize, usize)> {
        self.providers
            .iter()
            .enumerate()
            .flat_map(|(provider, p)| {
                p.data
                    .inputs
                    .iter()
                    .enumerate()
                    .filter_map(move |(slot, input)| {
                        (input.type_id == type_id && input.key == *key).then_some((provider, slot))
                    })
            })
            .collect()
    }

    /// 取得对应查询根的首个位置，没有直接查询时返回 dummy span。
    fn root(&self, type_id: usize) -> Span {
        self.requests
            .get(&self.types[type_id])
            .and_then(|spans| spans.first())
            .copied()
            .unwrap_or(rustc_span::DUMMY_SP)
    }

    /// 把消费者和输入请求信息补入诊断，帮助定位需求从何而来。
    fn usage_notes(&self, diagnostic: &mut Diagnostic, provider: usize, slot: usize) {
        let p = &self.providers[provider];
        let input = &p.data.inputs[slot];
        if input.optional {
            diagnostic.notes.push(
                "Option 只允许目标缺席；已存在的目标仍须满足唯一选择、无环和生命周期约束。".into(),
            );
        }
        if input.lazy {
            diagnostic
                .notes
                .push("此依赖是 lazy；延迟创建实例不会跳过依赖图验证。".into());
        }
        if p.data.lazy == Some(true) {
            diagnostic
                .notes
                .push("服务级 lazy 只影响自主预热；未查询的服务声明也必须通过完整图验证。".into());
        }
        if let ty::Adt(_, args) = self.types[p.data.type_id].kind()
            && !args.is_empty()
            && let Some(spans) = self.requests.get(&self.types[p.data.type_id])
        {
            for &span in spans.iter().take(3) {
                if source_span(self.tcx, span) != source_span(self.tcx, p.origin.input(slot)) {
                    diagnostic.labels.push((
                        span,
                        format!(
                            "此处使用 `{}`，触发该泛型服务的依赖检查",
                            self.service(provider)
                        ),
                    ));
                }
            }
        }
    }

    /// 把依赖证据链渲染为可审阅文本，不重新选择图边。
    fn edges(&self, edges: &[DependencyEdge]) -> String {
        let mut rows = Vec::new();
        for (index, edge) in edges.iter().enumerate() {
            if edges.len() > 8 && index == 4 {
                rows.push(format!(
                    "… 已省略 {} 条依赖关系，完整路径见 cause",
                    edges.len() - 8
                ));
            }
            if edges.len() > 8 && (4..edges.len() - 4).contains(&index) {
                continue;
            }
            let p = &self.providers[edge.consumer];
            let input = &p.data.inputs[edge.slot];
            let attributes = match (input.optional, input.lazy) {
                (true, true) => " [optional, lazy]",
                (true, false) => " [optional]",
                (false, true) => " [lazy]",
                _ => "",
            };
            rows.push(format!(
                "{}.{label}{attributes} → {} [{:?}]",
                self.service(edge.consumer),
                self.service(edge.target),
                self.providers[edge.target].data.lifetime,
                label = input.label
            ));
        }
        rows.join("\n")
    }

    /// 为证据中的依赖边追加用户输入位置标签。
    fn edge_labels(&self, diagnostic: &mut Diagnostic, edges: &[DependencyEdge]) {
        for edge in edges.iter().take(6) {
            diagnostic.labels.push((
                self.providers[edge.consumer].origin.input(edge.slot),
                format!(
                    "{} 依赖 {}",
                    self.input_name(edge.consumer, edge.slot),
                    self.service(edge.target)
                ),
            ));
        }
    }

    /// 按图错误种类构造代码、主位置、关联标签及完整 cause。
    fn render(&self, issue: &model::Diagnostic) -> Diagnostic {
        let mut result = match &issue.evidence {
            Evidence::MissingDependency { consumer, slot } => {
                let p = &self.providers[*consumer];
                let input = &p.data.inputs[*slot];
                // 只使用已经参与本轮分析的真实 provider/binding；诊断不新增候选或展开泛型。
                let alternatives: Vec<_> = self
                    .providers
                    .iter()
                    .enumerate()
                    .filter(|(_, candidate)| {
                        candidate.data.key != input.key
                            && (candidate.data.type_id == input.type_id
                                || self.bindings.iter().any(|binding| {
                                    binding.interface == self.types[input.type_id]
                                        && binding.concrete == self.types[candidate.data.type_id]
                                }))
                    })
                    .map(|(id, _)| id)
                    .collect();
                let mismatch = !alternatives.is_empty();
                let span = if mismatch {
                    p.origin
                        .input_keys
                        .get(slot)
                        .copied()
                        .unwrap_or(p.origin.input(*slot))
                } else {
                    p.origin.input(*slot)
                };
                let message = if mismatch {
                    format!(
                        "{} 请求{}，没有匹配的 `{}` 服务",
                        self.input_name(*consumer, *slot),
                        key_description(&input.key),
                        self.names[input.type_id]
                    )
                } else {
                    format!(
                        "无法注入 {}：没有匹配的 `{}` 服务声明",
                        self.input_name(*consumer, *slot),
                        self.names[input.type_id]
                    )
                };
                let mut d = Diagnostic::new(
                    if mismatch {
                        "NESTRS-DI002"
                    } else {
                        "NESTRS-DI001"
                    },
                    message,
                    span,
                );
                d.labels.push((
                    span,
                    if mismatch {
                        "这个 key 没有匹配的服务".into()
                    } else {
                        "这个必选依赖没有可用的服务声明".into()
                    },
                ));
                if mismatch {
                    for &id in alternatives.iter().take(6) {
                        let alternative = &self.providers[id];
                        d.labels.push((
                            alternative
                                .origin
                                .key
                                .unwrap_or(alternative.origin.declaration),
                            format!(
                                "{} 使用{}",
                                self.service(id),
                                key_description(&alternative.data.key)
                            ),
                        ));
                    }
                    d.notes.push(
                        "key 必须精确匹配；默认 key 不回退到其他 key，字符串与整数 key 也不相等。"
                            .into(),
                    );
                    d.help.push("若需要这里已有的实现，请统一消费方与声明方的 key；否则为请求的 key 声明服务。".into());
                } else {
                    if input.key != model::Key::Default {
                        d.notes
                            .push(format!("此次注入请求{}。", key_description(&input.key)));
                    }
                    if matches!(self.types[input.type_id].kind(), ty::Dynamic(..)) {
                        d.help.push(format!("声明实现 `{}` 的 injectable 或 factory 服务，并确保 key 匹配；普通 impl 会由工具自动绑定。", self.names[input.type_id]));
                    } else {
                        d.help.push(format!("为 `{}` 添加 `#[injectable]`，或声明返回该类型的 `#[factory]`，并确保 key 匹配。",self.names[input.type_id]));
                    }
                }
                self.usage_notes(&mut d, *consumer, *slot);
                d
            }
            Evidence::AmbiguousTrait {
                type_id,
                key,
                candidates,
            } => {
                let uses = self.uses(*type_id, key);
                let primary_count = candidates
                    .iter()
                    .filter(|&&id| self.providers[id].data.primary)
                    .count();
                let query = self.root(*type_id);
                let primary = uses
                    .first()
                    .map(|&(id, slot)| self.providers[id].origin.input(slot))
                    .unwrap_or_else(|| {
                        if !query.is_dummy() {
                            query
                        } else {
                            self.providers[candidates[0]].origin.declaration
                        }
                    });
                let subject = uses
                    .first()
                    .map(|&(id, slot)| self.input_name(id, slot))
                    .unwrap_or_else(|| {
                        if query.is_dummy() {
                            format!("已声明的 `{}` 接口", self.names[*type_id])
                        } else {
                            format!("对 `{}` 的查询", self.names[*type_id])
                        }
                    });
                let mut d = Diagnostic::new(
                    if primary_count > 1 {
                        "NESTRS-DI004"
                    } else {
                        "NESTRS-DI003"
                    },
                    format!(
                        "无法为 {subject} 选择唯一的 `{}` 实现",
                        self.names[*type_id]
                    ),
                    primary,
                );
                let reason = if primary_count > 1 {
                    format!("有 {primary_count} 个实现都被标记为 primary")
                } else {
                    format!("有 {} 个可用实现，但没有唯一 primary", candidates.len())
                };
                d.labels.push((primary, reason));
                for &id in candidates.iter().take(6) {
                    let p = &self.providers[id];
                    d.labels.push((
                        p.origin.primary.unwrap_or(p.origin.declaration),
                        format!(
                            "候选：{}{}",
                            self.service(id),
                            if p.data.primary { "（primary）" } else { "" }
                        ),
                    ));
                }
                if candidates.len() > 6 {
                    d.notes.push(format!(
                        "另有 {} 个候选，完整清单见 cause。",
                        candidates.len() - 6
                    ));
                }
                d.notes.push(format!(
                    "这些实现都使用{}；primary 只在同一 key 的候选中选择。",
                    key_description(key)
                ));
                for &(id, slot) in uses.iter().skip(1).take(3) {
                    d.labels.push((
                        self.providers[id].origin.input(slot),
                        "此处也请求同一接口与 key".into(),
                    ));
                }
                d.help.push(
                    if primary_count > 1 {
                        "只保留一个 primary，或给不同实现设置不同 key 并明确选择。"
                    } else {
                        "为其中一个实现添加 `#[primary]`，或分别设置 key 并在消费处明确选择。"
                    }
                    .into(),
                );
                if let Some(&(id, slot)) = uses.first() {
                    self.usage_notes(&mut d, id, slot);
                }
                d
            }
            Evidence::Cycle { edges } => {
                let edge = &edges[0];
                let mut d = Diagnostic::new(
                    "NESTRS-DI005",
                    format!("`{}` 的依赖形成循环", self.service(edge.consumer)),
                    self.providers[edge.consumer].origin.input(edge.slot),
                );
                self.edge_labels(&mut d, edges);
                d.notes.push(format!("依赖环：\n{}", self.edges(edges)));
                d.help.push("移除环中的一条依赖，或提取共同依赖的独立服务；改为 optional 或 lazy 不会消除已存在的环。".into());
                d
            }
            Evidence::ScopeRequired {
                singleton,
                scoped,
                edges,
            } => {
                let edge = edges.first();
                let p = &self.providers[*singleton];
                let span = edge
                    .map(|e| self.providers[e.consumer].origin.input(e.slot))
                    .unwrap_or(p.origin.declaration);
                let mut d = Diagnostic::new(
                    "NESTRS-DI006",
                    format!(
                        "`{}` 是 Singleton，但依赖链包含 Scoped 服务 `{}`",
                        self.service(*singleton),
                        self.service(*scoped)
                    ),
                    span,
                );
                self.edge_labels(&mut d, edges);
                let target = &self.providers[*scoped];
                d.labels.push((
                    target.origin.lifetime.unwrap_or(target.origin.declaration),
                    "此服务的生命周期是 Scoped".into(),
                ));
                d.notes.push(format!("依赖路径：\n{}", self.edges(edges)));
                if p.origin.lifetime.is_none() {
                    d.notes.push(format!(
                        "`{}` 未指定 lifetime，默认是 Singleton。",
                        self.service(*singleton)
                    ));
                }
                d.notes.push("Singleton 在 root 中构造；Transient、optional 和 lazy 都不会消除下游的 Scoped 要求。".into());
                d.help.push(format!("若 `{}` 属于请求，将其改为 Scoped；否则拆开应用级与请求级依赖。从 scope 查询 Singleton 不能解决此问题。",self.service(*singleton)));
                d
            }
            Evidence::DuplicateProvider { first, duplicate } => {
                let p = &self.providers[*duplicate];
                let other = &self.providers[*first];
                let mut d = Diagnostic::new(
                    "NESTRS-DI007",
                    format!(
                        "`{}` 在{} 下有多个创建声明",
                        self.service(*duplicate),
                        key_description(&p.data.key)
                    ),
                    p.origin.service,
                );
                d.labels.push((
                    p.origin.service,
                    format!("{} 提供这个类型", self.declaration(*duplicate)),
                ));
                d.labels.push((
                    other.origin.service,
                    format!("另一处声明：{}", self.declaration(*first)),
                ));
                d.notes.push(
                    "两个声明提供同一真实类型与 key；primary 不能覆盖重复的具体类型声明。".into(),
                );
                d.help
                    .push("保留一个创建声明，或为它们设置不同 key。".into());
                d
            }
            Evidence::DuplicateBinding { first, duplicate } => {
                let binding = &self.bindings[*duplicate];
                let span = self
                    .tcx
                    .def_span(binding.instance.def_id())
                    .source_callsite();
                let mut d =
                    Diagnostic::new("NESTRS-DI009", "同一服务与接口被显式绑定多次".into(), span);
                d.labels.push((span, "重复的显式绑定".into()));
                d.labels.push((
                    self.tcx
                        .def_span(self.bindings[*first].instance.def_id())
                        .source_callsite(),
                    "另一处绑定".into(),
                ));
                d.help.push(
                    "移除重复的隐藏 `#[bind]`；正常业务保留普通 impl，由工具按需求自动绑定。"
                        .into(),
                );
                d
            }
            Evidence::OrphanBinding { binding } => {
                let binding = &self.bindings[*binding];
                let span = self
                    .tcx
                    .def_span(binding.instance.def_id())
                    .source_callsite();
                let concrete = rustc_middle::ty::print::with_forced_trimmed_paths!(
                    binding.concrete.to_string()
                );
                let mut d = Diagnostic::new(
                    "NESTRS-DI010",
                    format!("显式绑定的 `{concrete}` 没有创建声明"),
                    span,
                );
                d.labels
                    .push((span, "绑定只声明接口投影，不负责创建服务".into()));
                d.help.push(format!("为 `{concrete}` 声明 injectable 或 factory；正常业务移除隐藏 `#[bind]`，使用普通 impl 自动绑定。"));
                d
            }
            Evidence::InvalidMetadata => {
                let mut d = Diagnostic::new(
                    "NESTRS-TOOL001",
                    "工具内部错误：依赖描述不一致".into(),
                    rustc_span::DUMMY_SP,
                );
                d.help.push(
                    "请保留诊断与工具版本用于排查；不要修改业务代码来满足内部槽位或类型 ID。"
                        .into(),
                );
                d
            }
        };
        result.cause = format!("{:?}\n{}", issue.kind, issue.message);
        result
    }
}
