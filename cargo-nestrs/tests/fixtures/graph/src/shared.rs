use nestrs::{factory, injectable};
use std::marker::PhantomData;

pub fn forbidden(phase: &str) -> ! {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, phase).unwrap();
    panic!("graph must not execute {phase}");
}

struct ExplosiveDefault;

impl Default for ExplosiveDefault {
    fn default() -> Self {
        forbidden("Default")
    }
}

fn forbidden_value() -> usize {
    forbidden("value")
}

#[injectable]
struct Database {
    default: ExplosiveDefault,
    #[value(forbidden_value())]
    value: usize,
}

pub trait Port: Send + Sync {
    fn ready(&self) -> bool;
}

struct Client;

impl Port for Client {
    fn ready(&self) -> bool {
        true
    }
}

async fn cleanup_client() {
    forbidden("cleanup");
}

#[factory(key = "active", cleanup = "cleanup_client")]
fn client() -> Client {
    forbidden("factory");
}

trait Missing: Send + Sync {}

#[injectable(lifetime = Scoped)]
struct Consumer {
    #[inject]
    database: Database,
    #[inject("active")]
    port: dyn Port,
    #[inject]
    optional: Option<dyn Missing>,
}

#[injectable]
pub struct Cache<T> {
    marker: PhantomData<T>,
    #[inject]
    database: Database,
}

pub fn query_only<T>() {
    // Keep the ordinary type parameter in an unrelated business function: roots
    // below still use explicit, independently nameable concrete arguments.
    let _ = std::marker::PhantomData::<T>;
}
