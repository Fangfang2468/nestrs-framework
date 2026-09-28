//! Read-only graph snapshots preserve complete dependency metadata without activation.
use nestrs::{factory, injectable};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

use nestrs_core::{ServiceProvider, get_required_service};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static TRANSIENTS: AtomicUsize = AtomicUsize::new(0);

fn construction() -> usize {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst)
}

#[injectable]
struct SharedDatabase {
    #[value(construction())]
    id: usize,
}

struct User;

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
    #[inject]
    database: SharedDatabase,
    #[value(construction())]
    id: usize,
}

trait Store: Send + Sync {
    fn database_id(&self) -> usize;
    fn id(&self) -> usize;
}

impl Store for Repository<User> {
    fn database_id(&self) -> usize {
        self.database.id
    }

    fn id(&self) -> usize {
        self.id
    }
}

#[injectable(lifetime = Transient)]
struct Formatter {
    #[value({ construction(); TRANSIENTS.fetch_add(1, Ordering::SeqCst) })]
    id: usize,
}

struct Channel {
    name: &'static str,
    id: usize,
}

#[factory(key = "replica")]
fn named_channel() -> Channel {
    Channel {
        name: "named",
        id: construction(),
    }
}

#[factory(key = 7)]
fn indexed_channel() -> Channel {
    Channel {
        name: "indexed",
        id: construction(),
    }
}

trait MissingPlugin: Send + Sync {}

#[injectable(lifetime = Scoped)]
struct GraphConsumer {
    #[inject]
    database: SharedDatabase,
    #[inject]
    store: dyn Store,
    #[inject]
    left: Formatter,
    #[inject]
    right: Formatter,
    #[inject(key = "replica")]
    named: Channel,
    #[inject(key = 7)]
    indexed: Channel,
    #[inject]
    plugin: Option<dyn MissingPlugin>,
    #[value(construction())]
    id: usize,
}

#[tokio::test]
async fn graph_snapshot_observes_static_graph_without_constructing_or_merging_transient_slots() {
    let graph = nestrs_core::__private::dependency_graph_json().unwrap();
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);
    let data: serde_json::Value = serde_json::from_str(&graph).unwrap();
    assert_eq!(data["version"], 1);
    let nodes = data["nodes"].as_array().unwrap();
    assert_eq!(nodes.len(), 6);
    let consumer_node = nodes
        .iter()
        .find(|node| node["label"] == "GraphConsumer")
        .unwrap();
    assert_eq!(consumer_node["lifetime"], "Scoped");
    let dependencies = consumer_node["dependencies"].as_array().unwrap();
    assert_eq!(dependencies.len(), 7);
    let dependency = |label: &str| {
        dependencies
            .iter()
            .find(|input| input["label"] == label)
            .unwrap()
    };

    let left = dependency("left");
    let right = dependency("right");
    assert_eq!(left["target"], right["target"]);
    assert_ne!(left["slot"], right["slot"]);
    let formatter = nodes
        .iter()
        .find(|node| node["id"] == left["target"])
        .unwrap();
    assert_eq!(formatter["label"], "Formatter");
    assert_eq!(formatter["lifetime"], "Transient");

    let store = dependency("store");
    assert!(store["requested"].as_str().unwrap().ends_with("::Store"));
    let repository = nodes
        .iter()
        .find(|node| node["id"] == store["target"])
        .unwrap();
    assert_eq!(repository["label"], "Repository<User>");
    assert_eq!(repository["lifetime"], "Singleton");
    assert_eq!(
        repository["dependencies"][0]["target"],
        dependency("database")["target"]
    );

    let named = dependency("named");
    let indexed = dependency("indexed");
    assert_eq!(
        named["key"],
        serde_json::json!({"kind": "named", "value": "replica"})
    );
    assert_eq!(
        indexed["key"],
        serde_json::json!({"kind": "indexed", "value": "7"})
    );
    assert_ne!(named["target"], indexed["target"]);
    for input in [named, indexed] {
        let channel = nodes
            .iter()
            .find(|node| node["id"] == input["target"])
            .unwrap();
        assert_eq!(channel["key"], input["key"]);
        assert_eq!(channel["kind"], "sync factory");
    }
    let optional = dependency("plugin");
    assert_eq!(optional["optional"], true);
    assert!(optional["target"].is_null());
    assert!(
        optional["requested"]
            .as_str()
            .unwrap()
            .ends_with("::MissingPlugin")
    );

    let provider = ServiceProvider::build().await.unwrap();
    let scope = provider.create_scope();
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 0);

    let consumer = get_required_service!(scope.service_provider(), GraphConsumer)
        .await
        .unwrap();
    assert_eq!(consumer.database.id, consumer.store.database_id());
    assert_ne!(consumer.id, consumer.store.id());
    assert_ne!(consumer.left.id, consumer.right.id);
    assert_eq!(TRANSIENTS.load(Ordering::SeqCst), 2);
    assert_eq!(consumer.named.name, "named");
    assert_eq!(consumer.indexed.name, "indexed");
    assert_ne!(consumer.named.id, consumer.indexed.id);
    assert!(consumer.plugin.is_none());
    assert_eq!(CONSTRUCTIONS.load(Ordering::SeqCst), 7);

    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(
        graph,
        nestrs_core::__private::dependency_graph_json().unwrap()
    );
}
