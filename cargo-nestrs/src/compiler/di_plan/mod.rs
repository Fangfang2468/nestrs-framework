//! 将 rustc 的真实声明身份编译成不可变的 DI 执行计划。
//!
//! 本模块只读取工具生成的无条件 metadata 标记，绝不执行业务构造或描述函数。
//! 实例化类型由 rustc 求解，候选/拓扑由纯模型算法决定；目标端仅装配真实 typed
//! adapter 地址与 TypeId。宿主地址、vtable 和 TypeId 数值不会跨编译目标序列化。

extern crate rustc_abi;
extern crate rustc_data_structures;
extern crate rustc_index;
extern crate rustc_infer;
extern crate rustc_trait_selection;

mod artifact;
mod emission;

use crate::autobind_semantic::{CompilerKey, external_key, provider_blueprint};
use crate::protocol::{InputPolicy, Lifetime, Marker, ProviderPolicy};
use crate::registration_codegen::{
    Kind, collect_callbacks, definition_path, descriptor_calls, reflect_item,
};
use cargo_nestrs::di_plan as model;
use rustc_hir::def_id::{DefId, LOCAL_CRATE};
use rustc_middle::{
    mir,
    ty::{self, Ty, TyCtxt, TypeVisitableExt},
};
use rustc_span::Span;
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

pub use emission::{enable, is_entry, prepare, provide};

struct Provider<'tcx> {
    instance: ty::Instance<'tcx>,
    data: model::Provider,
    kind: &'static str,
    source: Span,
}
struct Binding<'tcx> {
    instance: ty::Instance<'tcx>,
    concrete: Ty<'tcx>,
    interface: Ty<'tcx>,
}
struct Compiled<'tcx> {
    types: Vec<Ty<'tcx>>,
    providers: Vec<Provider<'tcx>>,
    bindings: Vec<Binding<'tcx>>,
    plan: model::Plan,
}

/// check 与 codegen 调用同一语义入口，因此 metadata-only 检查也会拒绝非法图。
/// library 只发布声明；完整验证发生在 binary/test 的最终组合处。
/// 此入口在 after_analysis 审计成功后计算并发布产物；MIR query 可更早消费另一份
/// 计算结果。暂保留两次计算，避免把 sidecar 发布提前到来源/两阶段一致性审计之前，
/// 也不将持有 Ty<'tcx> 的计划放入跨会话全局缓存。
pub fn validate(tcx: TyCtxt<'_>) -> Result<(), String> {
    if !entry(tcx) {
        return Ok(());
    }
    if !has_core(tcx) {
        // 完全没有服务的入口也有合法空图；不能因 rustc 未装载无用 extern 而漏产物。
        let plan = model::compile(&[], &[], &[]).expect("空图总是合法");
        return artifact::write(
            tcx,
            &Compiled {
                types: vec![],
                providers: vec![],
                bindings: vec![],
                plan,
            },
        );
    }
    let plan = compile(tcx)?;
    artifact::write(tcx, &plan)
}
fn entry(tcx: TyCtxt<'_>) -> bool {
    tcx.sess.opts.test
        || tcx
            .sess
            .opts
            .crate_types
            .contains(&rustc_session::config::CrateType::Executable)
}
fn has_core(tcx: TyCtxt<'_>) -> bool {
    tcx.crate_name(LOCAL_CRATE).as_str() == "nestrs_core"
        || tcx
            .crates(())
            .iter()
            .any(|&c| tcx.crate_name(c).as_str() == "nestrs_core")
}
fn name<'tcx>(tcx: TyCtxt<'tcx>, service: Ty<'tcx>) -> String {
    rustc_const_eval::util::type_name(tcx, service)
}
fn source(tcx: TyCtxt<'_>, span: Span) -> String {
    let pos = tcx
        .sess
        .source_map()
        .lookup_char_pos(span.source_callsite().lo());
    format!(
        "{}:{}:{}",
        pos.file.name.prefer_local_unconditionally(),
        pos.line,
        pos.col.0 + 1
    )
}
fn normalize<'tcx>(tcx: TyCtxt<'tcx>, service: Ty<'tcx>) -> Result<Ty<'tcx>, String> {
    // 有限声明闭包允许很深的普通 DAG，但不能允许 A<T> -> A<Vec<T>> 这类无限
    // 类型族耗尽编译器内存。检查的是单个类型表达式，不是服务图的路径深度。
    crate::query_roots::validate_type_complexity(tcx, service)?;
    let result = tcx
        .try_normalize_erasing_regions(
            ty::TypingEnv::fully_monomorphized(),
            ty::Unnormalized::new_wip(service),
        )
        .map_err(|error| format!("无法归一化 DI 类型 {service}: {error:?}"))?;
    if result.has_non_region_param() || result.has_infer() || result.has_escaping_bound_vars() {
        return Err(format!("DI 查询或描述中的类型尚未闭合：{result}"));
    }
    Ok(result)
}
fn intern<'tcx>(
    types: &mut Vec<Ty<'tcx>>,
    indices: &mut HashMap<Ty<'tcx>, usize>,
    ty: Ty<'tcx>,
) -> usize {
    *indices.entry(ty).or_insert_with(|| {
        let index = types.len();
        types.push(ty);
        index
    })
}
fn key(key: CompilerKey) -> model::Key {
    match key {
        CompilerKey::Default => model::Key::Default,
        CompilerKey::Named(s) => model::Key::Named(s),
        CompilerKey::Indexed(i) => model::Key::Indexed(i),
    }
}
fn const_number<'tcx>(_tcx: TyCtxt<'tcx>, constant: ty::Const<'tcx>) -> Result<u128, String> {
    constant
        .try_to_leaf()
        .map(|value| value.to_bits(value.size()))
        .ok_or_else(|| "DI 元数据 const 参数不是确定的标量".into())
}

fn text_literal<'tcx>(tcx: TyCtxt<'tcx>, operand: &mir::Operand<'tcx>) -> Result<String, String> {
    let mir::Operand::Constant(value) = operand else {
        return Err("DI 标签必须是工具生成的字面量".into());
    };
    let value = value
        .const_
        .eval(tcx, ty::TypingEnv::fully_monomorphized(), value.span)
        .map_err(|_| "DI 标签求值失败")?;
    let bytes = value
        .try_get_slice_bytes_for_diagnostics(tcx)
        .ok_or("DI 标签不是字符串")?;
    String::from_utf8(bytes.to_vec()).map_err(|_| "DI 标签不是 UTF-8".into())
}
fn read_provider<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: ty::Instance<'tcx>,
    types: &mut Vec<Ty<'tcx>>,
    indices: &mut HashMap<Ty<'tcx>, usize>,
) -> Result<Provider<'tcx>, String> {
    let mut identity = None;
    let mut inputs = Vec::new();
    let mut kind = "class";
    for call in descriptor_calls(tcx, instance)? {
        let (def, args, operands, body) =
            (call.definition, call.arguments, call.operands, call.body);
        if reflect_item(tcx, def, Marker::PlanProvider.name()) {
            let constants: Vec<_> = args
                .consts()
                .map(|c| const_number(tcx, c))
                .collect::<Result<_, _>>()?;
            let policy = ProviderPolicy::decode(&constants)?;
            let lifetime = match policy.lifetime {
                Lifetime::Singleton => model::Lifetime::Singleton,
                Lifetime::Scoped => model::Lifetime::Scoped,
                Lifetime::Transient => model::Lifetime::Transient,
            };
            // 初始化策略属于 Provider 声明；不能与输入槽位的 LAZY 边标记混合。
            let lazy = policy.initialization.lazy();
            let service = normalize(tcx, args.type_at(0))?;
            let id = intern(types, indices, service);
            let [value] = operands else {
                return Err("DI provider key metadata 版本不匹配".into());
            };
            if identity
                .replace((
                    id,
                    key(external_key(tcx, body, &value.node)?),
                    lifetime,
                    policy.primary,
                    lazy,
                ))
                .is_some()
            {
                return Err("同一描述回调有多个 provider 身份".into());
            }
        } else if reflect_item(tcx, def, Marker::PlanFactory.name()) {
            kind = if const_number(tcx, args.const_at(0))? != 0 {
                "async factory"
            } else {
                "sync factory"
            };
        } else if reflect_item(tcx, def, Marker::PlanInput.name()) {
            let constants: Vec<_> = args
                .consts()
                .map(|c| const_number(tcx, c))
                .collect::<Result<_, _>>()?;
            let policy = InputPolicy::decode(&constants)?;
            let [value, label] = operands else {
                return Err("DI input 字面量 metadata 版本不匹配".into());
            };
            inputs.push(model::Input {
                type_id: intern(types, indices, normalize(tcx, args.type_at(0))?),
                key: key(external_key(tcx, body, &value.node)?),
                slot: policy.slot,
                optional: policy.optional,
                lazy: policy.lazy,
                label: text_literal(tcx, &label.node)?,
            });
        }
    }
    let (type_id, key, lifetime, primary, lazy) = identity.ok_or_else(|| {
        format!(
            "{} 缺少完整 DI 计划元数据；请使用匹配的 cargo nestrs 重编译依赖",
            tcx.def_path_str(instance.def_id())
        )
    })?;
    inputs.sort_by_key(|input| input.slot);
    let span = tcx.def_span(instance.def_id());
    Ok(Provider {
        instance,
        data: model::Provider {
            type_id,
            key,
            lifetime,
            primary,
            lazy,
            source: source(tcx, span),
            inputs,
        },
        kind,
        source: span,
    })
}
fn read_binding<'tcx>(
    tcx: TyCtxt<'tcx>,
    instance: ty::Instance<'tcx>,
) -> Result<Binding<'tcx>, String> {
    for call in descriptor_calls(tcx, instance)? {
        let (def, args) = (call.definition, call.arguments);
        if reflect_item(tcx, def, Marker::Binding.name())
            || reflect_item(tcx, def, Marker::AutomaticBinding.name())
        {
            return Ok(Binding {
                instance,
                concrete: normalize(tcx, args.type_at(0))?,
                interface: normalize(tcx, args.type_at(1))?,
            });
        }
    }
    Err(format!(
        "{} 缺少 binding 类型标记",
        tcx.def_path_str(instance.def_id())
    ))
}
fn compile<'tcx>(tcx: TyCtxt<'tcx>) -> Result<Compiled<'tcx>, String> {
    let mut types = Vec::new();
    let mut indices = HashMap::new();
    let mut providers = Vec::new();
    let mut bindings = Vec::new();
    let mut passive = Vec::new();
    let mut requests: VecDeque<(Ty<'tcx>, Option<model::Key>)> = VecDeque::new();
    for (kind, callback) in collect_callbacks(tcx) {
        let instance = ty::Instance::mono(tcx, callback);
        match kind {
            Kind::Provider => {
                providers.push(read_provider(tcx, instance, &mut types, &mut indices)?)
            }
            Kind::Binding => bindings.push(read_binding(tcx, instance)?),
            Kind::AutomaticBinding => passive.push(read_binding(tcx, instance)?),
        }
    }
    for root in crate::query_roots::collect(tcx)? {
        requests.push_back((root.service, None));
    }
    for binding in &bindings {
        requests.push_back((binding.concrete, None));
    }
    let explicit: BTreeSet<_> = providers
        .iter()
        .map(|p| (p.data.type_id, p.data.key.clone()))
        .collect();
    let mut known = explicit.clone();
    let mut pairs: HashSet<_> = bindings.iter().map(|b| (b.concrete, b.interface)).collect();
    passive.retain(|b| pairs.insert((b.concrete, b.interface)));
    let mut expanded = HashSet::new();
    let mut interfaces = HashSet::new();
    let mut cursor = 0;
    let mut method_provider_count = usize::MAX;
    let mut method_roots = HashSet::new();
    // 同一个类型的多个key、重复Transient槽位仍完整留在 provider model 中；这里只将
    // 有限声明闭包去重。循环声明由下游 Kahn/显式栈诊断，不递归物化。
    loop {
        if types.len() > crate::query_roots::MAX_QUERY_TYPES {
            return Err(
                "DI 闭合类型集合超过编译分析上限，请检查持续增长的泛型查询或服务蓝图".into(),
            );
        }
        // 注入路径也能使泛型服务闭合。其 inherent/trait 方法内的查询与直接调用点
        // 使用同一摘要语义，新增查询还可能引出下一份 Provider，直到有限闭包稳定。
        if cursor == providers.len() && requests.is_empty() {
            if method_provider_count == providers.len() {
                break;
            }
            method_provider_count = providers.len();
            let known_types: Vec<_> = providers.iter().map(|p| types[p.data.type_id]).collect();
            for root in crate::query_roots::collect_with_providers(tcx, &known_types)? {
                if method_roots.insert(root.service) {
                    requests.push_back((root.service, None));
                }
            }
            if requests.is_empty() {
                break;
            }
        }
        if let Some((service, requested_key)) = requests.pop_front() {
            let id = intern(&mut types, &mut indices, service);
            if interfaces.insert(service) {
                let mut rest = Vec::new();
                for binding in passive.drain(..) {
                    if binding.interface == service {
                        requests.push_back((binding.concrete, None));
                        bindings.push(binding);
                    } else {
                        rest.push(binding);
                    }
                }
                passive = rest;
            }
            if requested_key
                .as_ref()
                .is_some_and(|k| explicit.contains(&(id, k.clone())))
            {
                continue;
            }
            if expanded.insert(service)
                && let Some(instance) = provider_blueprint(tcx, service)?
            {
                let provider = read_provider(tcx, instance, &mut types, &mut indices)?;
                if types[provider.data.type_id] != service {
                    return Err(format!("闭合蓝图 {service} 返回了错误的类型身份"));
                }
                if known.insert((provider.data.type_id, provider.data.key.clone())) {
                    providers.push(provider);
                }
            }
            continue;
        }
        for input in &providers[cursor].data.inputs {
            requests.push_back((types[input.type_id], Some(input.key.clone())));
        }
        cursor += 1;
    }
    providers.sort_by_key(|p| {
        (
            name(tcx, types[p.data.type_id]),
            p.data.key.clone(),
            p.data.source.clone(),
            tcx.def_path_str(p.instance.def_id()),
        )
    });
    bindings.sort_by_key(|b| {
        (
            name(tcx, b.interface),
            name(tcx, b.concrete),
            source(tcx, tcx.def_span(b.instance.def_id())),
        )
    });
    let binding_model: Vec<_> = bindings
        .iter()
        .map(|b| model::Binding {
            concrete: intern(&mut types, &mut indices, b.concrete),
            interface: intern(&mut types, &mut indices, b.interface),
            source: source(tcx, tcx.def_span(b.instance.def_id())),
        })
        .collect();
    let type_model: Vec<_> = types
        .iter()
        .enumerate()
        .map(|(id, ty)| model::Type {
            id,
            name: name(tcx, *ty),
        })
        .collect();
    let provider_model: Vec<_> = providers.iter().map(|p| p.data.clone()).collect();
    let mut plan =
        model::compile(&type_model, &provider_model, &binding_model).map_err(|diagnostics| {
            format!(
                "DI 依赖图编译失败（{} 项）\n{}",
                diagnostics.len(),
                diagnostics
                    .iter()
                    .map(|d| format!("- [{:?}] {}", d.kind, d.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        })?;

    // 完整候选集先经过图验证，才能裁掉不参与实际执行的投影。提前裁剪会掩盖
    // 重复显式 binding、缺失 concrete provider 或候选歧义；这些错误必须继续拒绝。
    // 输入与查询路由已经确定目标，core 只需要它们真正引用的类型化转换入口。
    let mut selected = vec![false; bindings.len()];
    for binding in plan
        .inputs
        .iter()
        .flatten()
        .filter_map(|input| input.binding)
        .chain(plan.routes.iter().filter_map(|route| route.binding))
    {
        selected[binding] = true;
    }
    let mut remap = vec![None; bindings.len()];
    let mut retained = Vec::new();
    for (old, binding) in bindings.into_iter().enumerate() {
        if selected[old] {
            // 沿已经排序的旧编号保留顺序，不让 HashMap/HashSet 枚举影响执行产物。
            remap[old] = Some(retained.len());
            retained.push(binding);
        }
    }
    for input in plan.inputs.iter_mut().flatten() {
        input.binding = input
            .binding
            .map(|old| remap[old].expect("已选输入的投影必须保留在最终执行计划中"));
    }
    for route in &mut plan.routes {
        route.binding = route
            .binding
            .map(|old| remap[old].expect("已选查询路由的投影必须保留在最终执行计划中"));
    }
    Ok(Compiled {
        types,
        providers,
        bindings: retained,
        plan,
    })
}
