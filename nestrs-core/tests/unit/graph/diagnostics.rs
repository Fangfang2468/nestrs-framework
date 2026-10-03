//! 验证只读图数据保留请求、槽位、key、来源与类型展示身份，不执行服务入口。

use ahash::AHashMap;
use std::any::TypeId;

use super::*;
use crate::{
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, FactoryFuture, FactoryInputs,
        InputSlot, project_required,
    },
    graph::{
        AbsentInput, CompiledDependency, CompiledNode, DependencyInput, NodePolicy, ProviderId,
    },
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};

fn must_not_construct(_: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    panic!("rendering HTML must not construct a service")
}

fn must_not_construct_sync(_: FactoryInputs<'_>) -> Result<ErasedService, ConstructionError> {
    panic!("rendering HTML must not invoke a factory")
}

fn must_not_construct_async(_: FactoryInputs<'_>) -> FactoryFuture<'_> {
    panic!("rendering HTML must not invoke an async factory")
}

fn common(lifetime: ServiceLifetime) -> NodePolicy {
    NodePolicy {
        lifetime,
        lazy: None,
        source: ServiceSource::new("src/services.rs", 17, 9),
        cleanup: None,
    }
}

fn identifier(name: &'static str) -> ServiceIdentifier {
    ServiceIdentifier::new(
        None,
        ServiceType {
            type_id: TypeId::of::<()>(),
            name,
        },
    )
}

fn dependency(
    slot: usize,
    label: &'static str,
    requested: &'static str,
    target: Option<ProviderId>,
) -> CompiledDependency {
    CompiledDependency {
        slot: InputSlot::new(slot),
        requested: identifier(requested),
        optional: target.is_none(),
        input: match target {
            Some(target) => DependencyInput::Immediate {
                target,
                project: project_required::<()>,
            },
            None => DependencyInput::Absent(AbsentInput::Immediate),
        },
        label: Some(label),
    }
}

fn node(name: &'static str, dependencies: Vec<CompiledDependency>) -> CompiledNode {
    CompiledNode {
        identifier: identifier(name),
        common: common(ServiceLifetime::Singleton),
        dependencies,
        constructor: Constructor::Class(must_not_construct),
        requires_scope: false,
    }
}

fn graph(nodes: Vec<CompiledNode>) -> ValidatedGraph {
    let mut dependents = vec![vec![]; nodes.len()];
    for (id, node) in nodes.iter().enumerate() {
        for dependency in &node.dependencies {
            if let Some(target) = dependency.input.target()
                && dependents[target].last() != Some(&id)
            {
                dependents[target].push(id);
            }
        }
    }
    ValidatedGraph {
        topological_order: (0..nodes.len()).rev().collect(),
        nodes,
        dependents,
        routes: AHashMap::new(),
    }
}

fn data(data: &str) -> Value {
    serde_json::from_str(data).unwrap()
}
fn render_json(graph: &ValidatedGraph) -> String {
    snapshot(graph).to_string()
}

#[test]
fn empty_graph_has_versioned_data() {
    assert_eq!(
        snapshot(&graph(vec![])),
        json!({ "version": 1, "nodes": [] })
    );
}

#[test]
fn preserves_provider_metadata_trait_selection_keys_and_optional_absence() {
    let mut payment = dependency(0, "payment", "dyn app::Payment", Some(1));
    payment.requested.service_key = Some(ServiceKey::Named("card".into()));
    payment.optional = true;
    let mut alternate = dependency(1, "alternate", "app::Client", Some(2));
    alternate.requested.service_key = Some(ServiceKey::Indexed(7));
    let mut fraud = dependency(2, "fraud", "dyn app::FraudCheck", None);
    fraud.requested.service_key = Some(ServiceKey::Named("regional".into()));
    let mut checkout = node("app::Checkout", vec![payment, alternate, fraud]);
    checkout.common.lifetime = ServiceLifetime::Scoped;
    checkout.requires_scope = true;
    let mut named = node("app::Client", vec![]);
    named.identifier.service_key = Some(ServiceKey::Named("card".into()));
    named.constructor = Constructor::Factory(FactoryInvoker::Sync(must_not_construct_sync));
    let mut indexed = node("app::Client", vec![]);
    indexed.identifier.service_key = Some(ServiceKey::Indexed(7));
    indexed.constructor = Constructor::Factory(FactoryInvoker::Async(must_not_construct_async));
    indexed.common.lifetime = ServiceLifetime::Transient;
    indexed.requires_scope = true;

    let output = data(&render_json(&graph(vec![checkout, named, indexed])));
    let nodes = output["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0]["id"], 1);
    assert_eq!(nodes[0]["name"], "app::Checkout");
    assert_eq!(nodes[0]["label"], "Checkout");
    assert_eq!(nodes[0]["key"], Value::Null);
    assert_eq!(nodes[0]["lifetime"], "Scoped");
    assert_eq!(nodes[0]["kind"], "class");
    assert_eq!(nodes[0]["requiresScope"], true);
    assert_eq!(
        nodes[0]["source"],
        json!({"file": "src/services.rs", "line": 17, "column": 9})
    );
    assert_eq!(
        nodes[0]["dependencies"][0],
        json!({
            "slot": 1,
            "label": "payment",
            "requested": "dyn app::Payment",
            "requestedLabel": "dyn Payment",
            "key": { "kind": "named", "value": "card" },
            "optional": true,
            "lazy": false,
            "target": 2,
        })
    );
    assert_eq!(nodes[0]["dependencies"][1]["target"], 3);
    assert_eq!(
        nodes[0]["dependencies"][1]["key"],
        json!({ "kind": "indexed", "value": "7" })
    );
    assert_eq!(nodes[0]["dependencies"][2]["target"], Value::Null);
    assert_eq!(nodes[0]["dependencies"][2]["optional"], true);
    assert_eq!(
        nodes[0]["dependencies"][2]["requested"],
        "dyn app::FraudCheck"
    );
    assert_eq!(nodes[1]["lifetime"], "Singleton");
    assert_eq!(nodes[1]["kind"], "sync factory");
    assert!(
        nodes[1].get("primary").is_none(),
        "运行计划不保留候选优先级"
    );
    assert_eq!(nodes[1]["key"], nodes[0]["dependencies"][0]["key"]);
    assert_eq!(nodes[2]["lifetime"], "Transient");
    assert_eq!(nodes[2]["kind"], "async factory");
    assert_eq!(nodes[2]["requiresScope"], true);
    assert_eq!(nodes[2]["key"], nodes[0]["dependencies"][1]["key"]);
}

#[test]
fn repeated_transient_consumptions_and_unused_providers_are_not_deduplicated() {
    let consumer = node(
        "app::Consumer",
        vec![
            dependency(0, "first", "app::Transient", Some(1)),
            dependency(1, "second", "app::Transient", Some(1)),
        ],
    );
    let mut transient = node("app::Transient", vec![]);
    transient.common.lifetime = ServiceLifetime::Transient;
    let output = data(&render_json(&graph(vec![
        consumer,
        transient,
        node("app::Unused", vec![]),
    ])));
    assert_eq!(output["nodes"].as_array().unwrap().len(), 3);
    let slots = output["nodes"][0]["dependencies"].as_array().unwrap();
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0]["target"], slots[1]["target"]);
    assert_eq!(slots[0]["slot"], 1);
    assert_eq!(slots[1]["slot"], 2);
    assert_eq!(slots[0]["label"], "first");
    assert_eq!(slots[1]["label"], "second");
    assert_eq!(output["nodes"][1]["lifetime"], "Transient");
    assert_eq!(output["nodes"][2]["name"], "app::Unused");
}

#[test]
fn indexed_keys_preserve_full_usize_precision_in_browser_json() {
    let mut service = node("app::Service", vec![]);
    service.identifier.service_key = Some(ServiceKey::Indexed(usize::MAX));
    let mut requested = dependency(0, "service", "app::Service", Some(1));
    requested.requested.service_key = Some(ServiceKey::Indexed(usize::MAX));
    let output = data(&render_json(&graph(vec![
        node("app::Consumer", vec![requested]),
        service,
    ])));
    let expected = json!({ "kind": "indexed", "value": usize::MAX.to_string() });
    assert_eq!(output["nodes"][1]["key"], expected);
    assert_eq!(output["nodes"][0]["dependencies"][0]["key"], expected);
}

#[test]
fn type_labels_handle_nested_unicode_generics_and_collisions_without_losing_full_names() {
    let output = data(&render_json(&graph(vec![
        node("billing::Service", vec![]),
        node("shipping::Service", vec![]),
        node("app::Cache<billing::User>", vec![]),
        node("app::Cache<shipping::User>", vec![]),
        node("类型::仓储<alloc::vec::Vec<领域::订单>>", vec![]),
    ])));
    for index in 0..4 {
        assert_eq!(
            output["nodes"][index]["label"],
            output["nodes"][index]["name"]
        );
    }
    assert_eq!(output["nodes"][4]["label"], "仓储<Vec<订单>>");
    assert_eq!(
        output["nodes"][4]["name"],
        "类型::仓储<alloc::vec::Vec<领域::订单>>"
    );
}

#[test]
fn metadata_round_trips_without_data_loss() {
    const UNTRUSTED: &str = "</script><script>alert(\"坏\")</script><!--&>\u{2028}\u{2029}\n\t\\";
    let mut service = node(UNTRUSTED, vec![dependency(0, UNTRUSTED, UNTRUSTED, None)]);
    service.identifier.service_key = Some(ServiceKey::Named(UNTRUSTED.into()));
    service.common.source = ServiceSource::new(UNTRUSTED, 21, 3);
    service.dependencies[0].requested.service_key = Some(ServiceKey::Named(UNTRUSTED.into()));
    let html = render_json(&graph(vec![service]));
    let output = data(&html);
    let service = &output["nodes"][0];
    assert_eq!(service["name"], UNTRUSTED);
    assert_eq!(service["key"]["value"], UNTRUSTED);
    assert_eq!(service["source"]["file"], UNTRUSTED);
    assert_eq!(service["dependencies"][0]["label"], UNTRUSTED);
    assert_eq!(service["dependencies"][0]["requested"], UNTRUSTED);
    assert_eq!(service["dependencies"][0]["key"]["value"], UNTRUSTED);
}

#[test]
fn ten_thousand_node_chain_is_flat_and_complete_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(|| {
            const COUNT: usize = 10_000;
            let nodes = (0..COUNT)
                .map(|id| {
                    let dependencies = if id + 1 < COUNT {
                        vec![dependency(0, "next", "app::Chain", Some(id + 1))]
                    } else {
                        vec![]
                    };
                    let mut node = node("app::Chain", dependencies);
                    node.identifier.service_key = Some(ServiceKey::Indexed(id));
                    node
                })
                .collect();
            let html = render_json(&graph(nodes));
            let output = data(&html);
            let nodes = output["nodes"].as_array().unwrap();
            assert_eq!(nodes.len(), COUNT);
            assert!(html.len() < COUNT * 1024);
            for (id, node) in nodes.iter().enumerate() {
                assert_eq!(node["id"], id + 1);
                let dependencies = node["dependencies"].as_array().unwrap();
                if id + 1 < COUNT {
                    assert_eq!(dependencies.len(), 1);
                    assert_eq!(dependencies[0]["target"], id + 2);
                    assert_eq!(dependencies[0]["slot"], 1);
                } else {
                    assert!(dependencies.is_empty());
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
