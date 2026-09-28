//! Offline graph presentation; runtime core only produces flat diagnostic data.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

pub fn render_html(data: &Value) -> Result<String, String> {
    match data.get("version").and_then(Value::as_u64) {
        Some(1) if data.get("nodes").is_some_and(Value::is_array) => {}
        Some(2) => validate_project(data)?,
        _ => return Err("unsupported Nestrs graph data version or shape".into()),
    }
    Ok(include_str!("graph.html").replacen(
        "/*NESTRS_GRAPH_DATA*/",
        &script_safe_json(&data.to_string()),
        1,
    ))
}

/// The project envelope is a presentation union. Each successful entry retains its
/// own local provider IDs; neither validation nor rendering merges its DI routes.
fn validate_project(data: &Value) -> Result<(), String> {
    let invalid = || "invalid Nestrs project graph data shape".to_owned();
    if data.get("kind").and_then(Value::as_str) != Some("project") {
        return Err(invalid());
    }
    let packages = data
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let entries = data
        .get("entries")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut package_names = HashMap::new();
    for package in packages {
        let id = string_field(package, "id").ok_or_else(invalid)?;
        let name = string_field(package, "name").ok_or_else(invalid)?;
        if id.is_empty()
            || name.is_empty()
            || string_field(package, "version").is_none()
            || package_names.insert(id, name).is_some()
        {
            return Err(invalid());
        }
    }
    let mut entry_ids = HashSet::new();
    for entry in entries {
        let id = string_field(entry, "id").ok_or_else(invalid)?;
        let package_id = string_field(entry, "packageId").ok_or_else(invalid)?;
        let package = string_field(entry, "package").ok_or_else(invalid)?;
        if id.is_empty()
            || !entry_ids.insert(id)
            || package_names.get(package_id).copied() != Some(package)
            || string_field(entry, "binary").is_none_or(str::is_empty)
            || !entry
                .get("requiredFeatures")
                .and_then(Value::as_array)
                .is_some_and(|features| features.iter().all(Value::is_string))
        {
            return Err(invalid());
        }
        match string_field(entry, "status") {
            Some("ok") => {
                if entry.get("diagnostic") != Some(&Value::Null) {
                    return Err(invalid());
                }
                validate_entry_graph(entry.get("graph").ok_or_else(invalid)?).map_err(
                    |reason| format!("invalid graph for project entry {id:?}: {reason}"),
                )?;
            }
            Some("error" | "skipped") => {
                if entry.get("graph") != Some(&Value::Null)
                    || string_field(entry, "diagnostic").is_none_or(str::is_empty)
                {
                    return Err(invalid());
                }
            }
            _ => return Err(invalid()),
        }
    }
    Ok(())
}

fn string_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn valid_number(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_u64)
        .is_some_and(|number| number <= 9_007_199_254_740_991)
}

fn valid_key(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Null) => true,
        Some(key) => {
            matches!(string_field(key, "kind"), Some("named" | "indexed"))
                && string_field(key, "value").is_some()
        }
        None => false,
    }
}

fn validate_entry_graph(graph: &Value) -> Result<(), &'static str> {
    if graph.get("version").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported graph version");
    }
    let nodes = graph
        .get("nodes")
        .and_then(Value::as_array)
        .ok_or("missing node list")?;
    let mut ids = HashSet::new();
    for node in nodes {
        if !valid_number(node.get("id"))
            || !ids.insert(node["id"].as_u64().ok_or("invalid node ID")?)
            || ["name", "label"]
                .iter()
                .any(|key| string_field(node, key).is_none())
            || !matches!(
                string_field(node, "lifetime"),
                Some("Singleton" | "Scoped" | "Transient")
            )
            || !matches!(
                string_field(node, "kind"),
                Some("class" | "sync factory" | "async factory")
            )
            || !valid_key(node.get("key"))
            || ["primary", "requiresScope"]
                .iter()
                .any(|key| !node.get(key).is_some_and(Value::is_boolean))
        {
            return Err("invalid or duplicate provider declaration");
        }
        let source = node.get("source").ok_or("missing source")?;
        if string_field(source, "file").is_none()
            || !valid_number(source.get("line"))
            || !valid_number(source.get("column"))
        {
            return Err("invalid provider source");
        }
    }
    for node in nodes {
        let dependencies = node
            .get("dependencies")
            .and_then(Value::as_array)
            .ok_or("missing input list")?;
        let mut slots = HashSet::new();
        for dependency in dependencies {
            if !valid_number(dependency.get("slot"))
                || !slots.insert(dependency["slot"].as_u64().ok_or("invalid slot")?)
                || ["label", "requested", "requestedLabel"]
                    .iter()
                    .any(|key| string_field(dependency, key).is_none())
                || !valid_key(dependency.get("key"))
                || !dependency.get("optional").is_some_and(Value::is_boolean)
            {
                return Err("invalid or duplicate input slot");
            }
            match dependency.get("target") {
                Some(Value::Null) if dependency["optional"] == true => {}
                Some(target) if target.as_u64().is_some_and(|id| ids.contains(&id)) => {}
                _ => return Err("input target is absent or outside its entry"),
            }
        }
    }
    Ok(())
}

/// application/json 的内容也由 HTML parser 识别结束标签，故不能仅依赖 JSON 引号转义。
fn script_safe_json(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '<' => escaped.push_str("\\u003c"),
            '>' => escaped.push_str("\\u003e"),
            '&' => escaped.push_str("\\u0026"),
            '\u{2028}' => escaped.push_str("\\u2028"),
            '\u{2029}' => escaped.push_str("\\u2029"),
            _ => escaped.push(character),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graph_page_is_offline_and_metadata_cannot_close_the_json_script() {
        let text = "</script><script>alert(1)</script>&\u{2028}\u{2029}";
        let data = serde_json::json!({"version":1,"nodes":[{"name":text}]});
        let html = render_html(&data).unwrap();
        assert!(html.to_ascii_lowercase().starts_with("<!doctype html>"));
        assert!(!html.contains("<script src="));
        assert!(!html.contains("<script>alert"));
        let start = html.find("id=\"graph-data\"").unwrap();
        let start = start + html[start..].find('>').unwrap() + 1;
        let end = start + html[start..].find("</script>").unwrap();
        let encoded = &html[start..end];
        assert!(encoded.contains("\\u003c/script\\u003e"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(encoded).unwrap(),
            data
        );
    }
    #[test]
    fn unsupported_graph_versions_are_rejected() {
        assert!(render_html(&serde_json::json!({"version":2,"nodes":[]})).is_err());
        assert!(render_html(&serde_json::json!({"version":1,"nodes":null})).is_err());
    }
    fn project_entry(id: &str) -> Value {
        serde_json::json!({
            "id": id, "packageId": "package-id", "package": "application", "binary": id,
            "status": "ok", "diagnostic": null, "requiredFeatures": [],
            "graph": { "version": 1, "nodes": [{
                "id": 1, "name": "app::Service", "label": "Service",
                "lifetime": "Singleton", "kind": "class", "key": null,
                "primary": false, "requiresScope": false,
                "source": {"file": "src/lib.rs", "line": 3, "column": 1},
                "dependencies": []
            }] }
        })
    }

    fn project_data() -> Value {
        serde_json::json!({
            "version": 2, "kind": "project",
            "packages": [{"id": "package-id", "name": "application", "version": "0.1.0"}],
            "entries": [project_entry("first"), project_entry("second")]
        })
    }

    #[test]
    fn project_entries_keep_independent_local_ids_and_failed_entries() {
        let mut data = project_data();
        assert!(
            render_html(&data).is_ok(),
            "local provider ID 1 may occur in each entry"
        );
        data["entries"][1]["status"] = "error".into();
        data["entries"][1]["graph"] = Value::Null;
        data["entries"][1]["diagnostic"] = "graph validation failed: missing service".into();
        assert!(render_html(&data).is_ok());
        data["entries"][0]["status"] = "skipped".into();
        data["entries"][0]["graph"] = Value::Null;
        data["entries"][0]["diagnostic"] = "requires feature payments".into();
        data["entries"][0]["requiredFeatures"] = serde_json::json!(["payments"]);
        assert!(
            render_html(&data).is_ok(),
            "all-error/skipped reports remain viewable"
        );
    }

    #[test]
    fn project_schema_rejects_ambiguous_entries_and_invalid_status_payloads() {
        for (pointer, replacement) in [
            ("/kind", serde_json::json!("other")),
            ("/entries/1/id", serde_json::json!("first")),
            ("/entries/1/packageId", serde_json::json!("unknown")),
            ("/entries/1/package", serde_json::json!("wrong-name")),
            ("/entries/1/status", serde_json::json!("pending")),
            ("/entries/1/status", serde_json::json!("error")),
            ("/entries/1/diagnostic", serde_json::json!("unexpected")),
            ("/entries/1/requiredFeatures", serde_json::json!([3])),
            (
                "/entries/1/graph/nodes/0/id",
                serde_json::json!(9_007_199_254_740_992_u64),
            ),
            ("/entries/1/graph/nodes/0/source", Value::Null),
            ("/entries/1/graph/nodes/0/dependencies", Value::Null),
        ] {
            let mut data = project_data();
            *data.pointer_mut(pointer).unwrap() = replacement;
            assert!(
                render_html(&data).is_err(),
                "accepted malformed field {pointer}"
            );
        }
    }

    #[test]
    fn input_routes_must_stay_inside_their_own_entry() {
        let mut data = project_data();
        data["entries"][1]["graph"]["nodes"][0]["id"] = 2.into();
        let dependency = serde_json::json!({
            "slot": 1, "label": "service", "requested": "app::Service",
            "requestedLabel": "Service", "optional": false, "key": null, "target": 2
        });
        data["entries"][0]["graph"]["nodes"][0]["dependencies"] = serde_json::json!([dependency]);
        assert!(
            render_html(&data)
                .unwrap_err()
                .contains("outside its entry")
        );
        data["entries"][0]["graph"]["nodes"][0]["dependencies"][0]["target"] = Value::Null;
        assert!(
            render_html(&data).is_err(),
            "a required input cannot be absent"
        );
        data["entries"][0]["graph"]["nodes"][0]["dependencies"][0]["optional"] = true.into();
        assert!(render_html(&data).is_ok());
    }

    #[test]
    fn project_metadata_and_diagnostics_cannot_inject_html() {
        let mut data = project_data();
        let attack = "</script><img src=x onerror=alert(1)>&\u{2028}\u{2029}";
        data["packages"][0]["name"] = attack.into();
        for entry in data["entries"].as_array_mut().unwrap() {
            entry["package"] = attack.into();
        }
        data["entries"][1]["status"] = "error".into();
        data["entries"][1]["graph"] = Value::Null;
        data["entries"][1]["diagnostic"] = attack.into();
        data["entries"][0]["graph"]["nodes"][0]["source"]["file"] = attack.into();
        let rendered = render_html(&data).unwrap();
        assert!(!rendered.contains("<img src=x"));
        assert!(!rendered.contains("<script src="));
        let marker = "<script type=\"application/json\" id=\"graph-data\">";
        let encoded = rendered
            .split_once(marker)
            .unwrap()
            .1
            .split_once("</script>")
            .unwrap()
            .0;
        assert_eq!(serde_json::from_str::<Value>(encoded).unwrap(), data);
    }
}
