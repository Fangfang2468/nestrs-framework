//! graph 命令直接消费的编译产物。它与执行计划来自同一个已验证模型，不运行目标程序。

use super::*;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

pub(super) fn write<'tcx>(tcx: TyCtxt<'tcx>, plan: &Compiled<'tcx>) -> Result<(), String> {
    write_reflection(tcx, plan)?;
    let Some(_) = std::env::var_os("NESTRS_GRAPH_PLAN") else {
        return Ok(());
    };
    let Some(target) = std::env::var_os("NESTRS_GRAPH_TARGET") else {
        return Ok(());
    };
    if tcx.crate_name(LOCAL_CRATE).as_str() != target.to_string_lossy() {
        return Ok(());
    }
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or("graph入口缺少Cargo manifest路径")?;
    let expected = std::env::var_os("NESTRS_GRAPH_MANIFEST")
        .map(PathBuf::from)
        .ok_or("graph命令缺少manifest身份")?;
    if !same(&manifest, &expected) {
        return Ok(());
    }
    let source = std::env::var_os("NESTRS_GRAPH_SOURCE")
        .map(PathBuf::from)
        .ok_or("graph命令缺少source身份")?;
    let actual = tcx
        .sess
        .local_crate_source_file()
        .ok_or("graph编译缺少源码路径")?;
    if !crate::graph_entry::matches_target(
        tcx.crate_name(LOCAL_CRATE).as_str(),
        actual.local_path().ok_or("graph源码无本地路径")?,
    )? {
        return Ok(());
    }
    let metadata = match tcx
        .output_filenames(())
        .path(rustc_session::config::OutputType::Metadata)
    {
        rustc_session::config::OutFileName::Real(path) => path,
        rustc_session::config::OutFileName::Stdout => {
            return Err("graph metadata 必须写入文件".into());
        }
    };
    let output = metadata.with_extension("nestrs-plan.json");
    let graph = snapshot(tcx, plan);
    let result = json!({"version":1,"binary":std::env::var("NESTRS_GRAPH_BINARY").map_err(|e|e.to_string())?,"crate":tcx.crate_name(LOCAL_CRATE).as_str(),"manifest":manifest,"source":source,"graph":graph,"metadata":metadata});
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = output.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(
        &temporary,
        serde_json::to_vec(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    // Windows rename 不能覆盖已有文件；graph独立构建单元串行更新同一产物。
    if output.exists() {
        std::fs::remove_file(&output).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&temporary, &output).map_err(|e| e.to_string())
}
fn same(a: &Path, b: &Path) -> bool {
    matches!((a.canonicalize(),b.canonicalize()),(Ok(a),Ok(b)) if a==b)
}
fn snapshot<'tcx>(tcx: TyCtxt<'tcx>, plan: &Compiled<'tcx>) -> Value {
    let full: Vec<_> = plan.types.iter().map(|ty| name(tcx, *ty)).collect();
    let mut groups = BTreeMap::<String, BTreeSet<&str>>::new();
    for name in &full {
        groups.entry(short_name(name)).or_default().insert(name);
    }
    let mut labels = BTreeMap::new();
    for (short, group) in groups {
        let collision = group.len() > 1;
        for name in group {
            labels.insert(
                name,
                if collision {
                    name.to_owned()
                } else {
                    short.clone()
                },
            );
        }
    }
    let nodes:Vec<_>=plan.providers.iter().enumerate().map(|(id,p)| {
        let loc=tcx.sess.source_map().lookup_char_pos(p.source.source_callsite().lo());
        let dependencies:Vec<_>=p.data.inputs.iter().enumerate().map(|(slot,input)|json!({
            "slot":input.slot+1,"label":input.label,"requested":full[input.type_id],"requestedLabel":labels[full[input.type_id].as_str()],
            "key":key_value(&input.key),"optional":input.optional,"lazy":input.lazy,"target":plan.plan.inputs[id][slot].target.map(|t|t+1)
        })).collect();
        json!({"id":id+1,"name":full[p.data.type_id],"label":labels[full[p.data.type_id].as_str()],
            "lifetime":format!("{:?}",p.data.lifetime),"kind":p.kind,"key":key_value(&p.data.key),"primary":p.data.primary,
            "initialization":initialization(p.data.lazy),
            "requiresScope":plan.plan.requires_scope[id],"source":{"file":loc.file.name.prefer_local_unconditionally().to_string(),"line":loc.line,"column":loc.col.0+1},"dependencies":dependencies})
    }).collect();
    json!({"version":1,"nodes":nodes})
}
/// 节点初始化策略与 dependencies[].lazy 的边语义分别展示，避免把跳过预热画成代理注入。
fn initialization(lazy: Option<bool>) -> &'static str {
    match lazy {
        None => "inherit",
        Some(true) => "lazy",
        Some(false) => "eager",
    }
}
fn key_value(key: &model::Key) -> Value {
    match key {
        model::Key::Default => Value::Null,
        model::Key::Named(v) => json!({"kind":"named","value":v}),
        model::Key::Indexed(v) => json!({"kind":"indexed","value":v.to_string()}),
    }
}

/// 仅缩短展示标签，不参与类型身份比较；平面扫描可处理深层泛型而不递归。
fn short_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut characters = name.char_indices().peekable();
    while let Some((start, character)) = characters.next() {
        if character.is_alphanumeric() || character == '_' {
            let mut segment_start = start;
            let mut end = start + character.len_utf8();
            while let Some(&(index, next)) = characters.peek() {
                if next.is_alphanumeric() || next == '_' {
                    characters.next();
                    end = index + next.len_utf8();
                } else if next == ':' && name[index..].starts_with("::") {
                    let after = index + 2;
                    if name[after..]
                        .chars()
                        .next()
                        .is_some_and(|n| n.is_alphanumeric() || n == '_')
                    {
                        characters.next();
                        characters.next();
                        segment_start = after;
                        end = after;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            result.push_str(&name[segment_start..end]);
        } else {
            result.push(character);
        }
    }
    result
}

/// 每个最终入口保存自己的冻结执行清单，供开发者审阅与工具诊断。真实构造代码仍在
/// 类型所属 crate 内按普通 Rust 编译；这里记录的是已选适配器及节点编号，不把候选
/// 集、泛型蓝图或依赖请求重新包装成运行期注册库。文件不参与程序运行或计划装载。
fn write_reflection<'tcx>(tcx: TyCtxt<'tcx>, plan: &Compiled<'tcx>) -> Result<(), String> {
    let metadata = match tcx
        .output_filenames(())
        .path(rustc_session::config::OutputType::Metadata)
    {
        rustc_session::config::OutFileName::Real(path) => path,
        rustc_session::config::OutFileName::Stdout => return Ok(()),
    };
    let output = metadata.with_extension("nestrs-reflect.json");
    let options = crate::registration_codegen::startup_options(tcx.sess);
    let nodes: Vec<_> = plan.providers.iter().enumerate().map(|(id, provider)| {
        let inputs: Vec<_> = plan.plan.inputs[id].iter().enumerate().map(|(slot, input)| {
            let declaration = &provider.data.inputs[slot];
            json!({"slot": slot, "target": input.target, "projection": input.binding,
                "optional": declaration.optional, "lazy": declaration.lazy,
                "type": name(tcx, plan.types[declaration.type_id]),
                "key": key_value(&declaration.key), "label": declaration.label})
        }).collect();
        json!({"id": id, "type": name(tcx, plan.types[provider.data.type_id]),
            "adapter": tcx.def_path_str(provider.instance.def_id()),
            "adapterCrate": tcx.crate_name(provider.instance.def_id().krate).as_str(),
            "key": key_value(&provider.data.key), "lifetime": format!("{:?}", provider.data.lifetime),
            "initialization": initialization(provider.data.lazy), "requiresScope": plan.plan.requires_scope[id],
            "source": source(tcx, provider.source), "inputs": inputs})
    }).collect();
    let projections: Vec<_> = plan.bindings.iter().enumerate().map(|(id, binding)| json!({
        "id": id, "concrete": name(tcx, binding.concrete), "interface": name(tcx, binding.interface),
        "adapter": tcx.def_path_str(binding.instance.def_id()),
        "adapterCrate": tcx.crate_name(binding.instance.def_id().krate).as_str(),
    })).collect();
    let routes: Vec<_> = plan
        .plan
        .routes
        .iter()
        .map(|route| {
            json!({
                "type": name(tcx, plan.types[route.type_id]), "key": key_value(&route.key),
                "provider": route.provider, "projection": route.binding,
            })
        })
        .collect();
    let result = json!({"format": "nestrs-reflect", "version": 1,
        "entry": crate::protocol::PLAN_ENTRY, "crate": tcx.crate_name(LOCAL_CRATE).as_str(),
        "target": tcx.sess.opts.target_triple.to_string(),
        "initialization": if options.eager {"eager"} else {"lazy"},
        "maxConcurrentActivations": options.max_concurrent_activations,
        "nodes": nodes, "projections": projections, "routes": routes,
        "order": plan.plan.order, "dependents": plan.plan.dependents});
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let temporary = output.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(
        &temporary,
        serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("无法写入反射执行清单 {}：{e}", output.display()))?;
    if output.exists() {
        std::fs::remove_file(&output).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&temporary, &output).map_err(|e| e.to_string())
}
