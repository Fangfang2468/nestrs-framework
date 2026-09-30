//! 自包含图页面的数据边界。只读取冻结元数据，不访问实例或调用任何服务入口。

use serde_json::{Value, json};

use super::{Constructor, ValidatedGraph, names::display_names};
use crate::{ServiceKey, ServiceLifetime, registration::provider::FactoryInvoker};

/// 图以平坦的声明列表和目标编号编码；深依赖链不会形成嵌套 JSON 或递归遍历。
pub(crate) fn snapshot(graph: &ValidatedGraph) -> Value {
    let names = display_names(graph);
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(id, node)| {
            let dependencies: Vec<_> = node
                .dependencies
                .iter()
                .map(|dependency| {
                    json!({
                        "slot": dependency.slot.index() + 1,
                        "label": dependency.label.unwrap_or("input"),
                        "requested": dependency.requested.service_type.name,
                        "requestedLabel": names[dependency.requested.service_type.name],
                        "key": key_value(dependency.requested.service_key.as_ref()),
                        "optional": dependency.optional,
                        "target": dependency.target.map(|target| target + 1),
                    })
                })
                .collect();
            let lifetime = match node.common.lifetime {
                ServiceLifetime::Singleton => "Singleton",
                ServiceLifetime::Scoped => "Scoped",
                ServiceLifetime::Transient => "Transient",
            };
            let kind = match node.constructor {
                Constructor::Class(_) => "class",
                Constructor::Factory(FactoryInvoker::Sync(_)) => "sync factory",
                Constructor::Factory(FactoryInvoker::Async(_)) => "async factory",
            };
            json!({
                "id": id + 1,
                "name": node.identifier.service_type.name,
                "label": names[node.identifier.service_type.name],
                "lifetime": lifetime,
                "kind": kind,
                "key": key_value(node.identifier.service_key.as_ref()),
                "primary": node.common.primary,
                "requiresScope": node.requires_scope,
                "source": {
                    "file": node.common.source.file,
                    "line": node.common.source.line,
                    "column": node.common.source.column,
                },
                "dependencies": dependencies,
            })
        })
        .collect();
    json!({ "version": 1, "nodes": nodes })
}

fn key_value(key: Option<&ServiceKey>) -> Value {
    match key {
        Some(ServiceKey::Named(name)) => json!({ "kind": "named", "value": name }),
        // JavaScript 数值不能精确表示所有 usize；编号 key 使用十进制字符串无损输出。
        Some(ServiceKey::Indexed(index)) => {
            json!({ "kind": "indexed", "value": index.to_string() })
        }
        None => Value::Null,
    }
}

#[cfg(test)]
#[path = "../../tests/unit/graph/diagnostics.rs"]
mod tests;
