use crate::*;
use nestrs::{factory, injectable};
use nestrs_core::{ServiceProvider, ServiceScope};

#[injectable]
struct Base {
    #[value(17)]
    value: usize,
}
trait Value: Send + Sync {
    fn value(&self) -> usize;
}
impl Value for Base {
    fn value(&self) -> usize {
        self.value
    }
}
struct Missing;
#[injectable]
struct LazyBase {
    #[value(17)]
    value: usize,
}

#[injectable(lifetime=Transient)]
struct Mixed {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    a: Base,
    #[inject]
    b: Base,
    #[inject]
    c: dyn Value,
    #[inject]
    d: dyn Value,
    #[inject]
    optional: Option<Base>,
    #[inject]
    absent: Option<Missing>,
    #[inject]
    #[lazy]
    l0: LazyBase,
    #[inject]
    #[lazy]
    l1: LazyBase,
}
#[injectable(lifetime=Scoped)]
struct Scoped {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    a: Base,
    #[inject]
    b: Base,
    #[inject]
    c: dyn Value,
    #[inject]
    d: dyn Value,
    #[inject]
    optional: Option<Base>,
    #[inject]
    absent: Option<Missing>,
    #[inject]
    #[lazy]
    l0: LazyBase,
    #[inject]
    #[lazy]
    l1: LazyBase,
}
#[injectable(lifetime=Transient)]
struct Lazy4 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    #[lazy]
    a: LazyBase,
    #[inject]
    #[lazy]
    b: LazyBase,
    #[inject]
    #[lazy]
    c: LazyBase,
    #[inject]
    #[lazy]
    d: LazyBase,
}
struct Async4 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime=Transient)]
async fn make_async(a: Base, b: Base, c: Base, d: Base) -> Async4 {
    tokio::task::yield_now().await;
    Async4 {
        value: a.value + b.value + c.value + d.value,
        _probe: Probe::new(),
    }
}
struct Payload {
    bytes: Vec<u8>,
    _probe: Probe,
}
#[factory(lifetime=Scoped)]
fn make_payload() -> Payload {
    Payload {
        bytes: vec![0x5a; PAYLOAD_BYTES.load(Ordering::Relaxed)],
        _probe: Probe::new(),
    }
}
async fn cleanup_sparse() {
    tokio::task::yield_now().await;
    CLEANED.fetch_add(1, Ordering::Relaxed);
}
#[injectable(lifetime=Scoped,cleanup="cleanup_sparse")]
struct SparseLeaf {
    #[value(17)]
    value: usize,
    #[value(Probe::new())]
    _probe: Probe,
}
#[injectable(lifetime=Scoped,cleanup="cleanup_sparse")]
struct Sparse {
    #[inject]
    #[lazy]
    leaf: SparseLeaf,
    #[value(Probe::new())]
    _probe: Probe,
}

#[cfg(feature = "large-graph")]
include!("padding.rs");

pub struct Root(ServiceProvider);
pub struct Scope<'a>(ServiceScope<'a>);
impl Root {
    pub async fn new() -> Self {
        Self(ServiceProvider::build().await.unwrap())
    }
    pub fn scope(&self) -> Scope<'_> {
        Scope(self.0.create_scope())
    }
    pub async fn settle(&self) {
        self.0.get_required_service::<Base>().await.unwrap();
        tokio::task::yield_now().await;
    }
    pub async fn close(self) {
        self.0.dispose_async().await.unwrap();
    }
}
impl Scope<'_> {
    pub async fn query(&mut self, scenario: &str) -> usize {
        let p = self.0.service_provider();
        match scenario {
            "mixed" => {
                let s = black_box(p.get_required_service::<Mixed>().await.unwrap());
                s.a.value
                    + s.b.value
                    + s.c.value()
                    + s.d.value()
                    + s.optional.as_ref().unwrap().value
                    + usize::from(s.absent.is_some())
                    + s.l0.get().await.unwrap().value
                    + s.l1.get().await.unwrap().value
            }
            "scoped" => {
                let s = black_box(p.get_required_service::<Scoped>().await.unwrap());
                s.a.value
                    + s.b.value
                    + s.c.value()
                    + s.d.value()
                    + s.optional.as_ref().unwrap().value
                    + usize::from(s.absent.is_some())
                    + s.l0.get().await.unwrap().value
                    + s.l1.get().await.unwrap().value
            }
            "lazy4" => {
                let s = black_box(p.get_required_service::<Lazy4>().await.unwrap());
                s.a.get().await.unwrap().value
                    + s.b.get().await.unwrap().value
                    + s.c.get().await.unwrap().value
                    + s.d.get().await.unwrap().value
            }
            "async4" => black_box(p.get_required_service::<Async4>().await.unwrap()).value,
            "payload" => {
                let s = black_box(p.get_required_service::<Payload>().await.unwrap());
                assert_eq!(s.bytes.first(), Some(&0x5a));
                assert_eq!(s.bytes.last(), Some(&0x5a));
                black_box(s.bytes.len())
            }
            "sparse" => {
                let s = black_box(p.get_required_service::<Sparse>().await.unwrap());
                s.leaf.get().await.unwrap().value
            }
            _ => panic!("scenario"),
        }
    }
    pub async fn close(self) {
        self.0.dispose_async().await.unwrap();
    }
}
