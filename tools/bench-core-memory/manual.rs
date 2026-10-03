//! Explicit composition baseline: same values/ownership duration, fewer runtime guarantees.
use crate::*;
use std::sync::OnceLock;
struct Base {
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
struct Mixed {
    a: Arc<Base>,
    b: Arc<Base>,
    c: Arc<dyn Value>,
    d: Arc<dyn Value>,
    optional: Option<Arc<Base>>,
    absent: Option<Arc<Missing>>,
    lazy: [OnceLock<Arc<Base>>; 2],
    _probe: Probe,
}
impl Mixed {
    fn new(root: &Root) -> Self {
        Self {
            a: root.base.clone(),
            b: root.base.clone(),
            c: root.base.clone(),
            d: root.base.clone(),
            optional: Some(root.base.clone()),
            absent: None,
            lazy: std::array::from_fn(|_| OnceLock::new()),
            _probe: Probe::new(),
        }
    }
    fn value(&self, root: &Root) -> usize {
        self.a.value
            + self.b.value
            + self.c.value()
            + self.d.value()
            + self.optional.as_ref().unwrap().value
            + usize::from(self.absent.is_some())
            + self
                .lazy
                .iter()
                .map(|l| l.get_or_init(|| root.lazy()).value)
                .sum::<usize>()
    }
}
struct Lazy4 {
    lazy: [OnceLock<Arc<Base>>; 4],
    _probe: Probe,
}
struct Async4 {
    value: usize,
    _probe: Probe,
}
struct Payload {
    bytes: Vec<u8>,
    _probe: Probe,
}
struct SparseLeaf {
    value: usize,
    _probe: Probe,
}
struct Sparse {
    leaf: OnceLock<Arc<SparseLeaf>>,
    _probe: Probe,
}
pub struct Root {
    base: Arc<Base>,
    lazy: OnceLock<Arc<Base>>,
}
pub struct Scope<'a> {
    root: &'a Root,
    mixed: Vec<Arc<Mixed>>,
    lazy: Vec<Arc<Lazy4>>,
    asyncs: Vec<Arc<Async4>>,
    scoped: Option<Arc<Mixed>>,
    payload: Option<Arc<Payload>>,
    sparse: Option<Arc<Sparse>>,
}
impl Root {
    pub async fn new() -> Self {
        Self {
            base: Arc::new(Base { value: 17 }),
            lazy: OnceLock::new(),
        }
    }
    fn lazy(&self) -> Arc<Base> {
        self.lazy
            .get_or_init(|| Arc::new(Base { value: 17 }))
            .clone()
    }
    pub async fn scope(&self) -> Scope<'_> {
        Scope {
            root: self,
            mixed: Vec::new(),
            lazy: Vec::new(),
            asyncs: Vec::new(),
            scoped: None,
            payload: None,
            sparse: None,
        }
    }
    pub async fn settle(&self) {
        black_box(&self.base);
        tokio::task::yield_now().await;
    }
    pub async fn close(self) {
        drop(self);
    }
}
impl Scope<'_> {
    pub async fn query(&mut self, scenario: &str) -> usize {
        match scenario {
            "mixed" => {
                let s = Arc::new(Mixed::new(self.root));
                let value = black_box(&s).value(self.root);
                self.mixed.push(s);
                value
            }
            "scoped" => {
                let s = self
                    .scoped
                    .get_or_insert_with(|| Arc::new(Mixed::new(self.root)));
                black_box(s).value(self.root)
            }
            "lazy4" => {
                let s = Arc::new(Lazy4 {
                    lazy: std::array::from_fn(|_| OnceLock::new()),
                    _probe: Probe::new(),
                });
                let value = black_box(&s)
                    .lazy
                    .iter()
                    .map(|l| l.get_or_init(|| self.root.lazy()).value)
                    .sum();
                self.lazy.push(s);
                value
            }
            "async4" => {
                let a = &self.root.base;
                let b = &self.root.base;
                let c = &self.root.base;
                let d = &self.root.base;
                tokio::task::yield_now().await;
                let s = Arc::new(Async4 {
                    value: a.value + b.value + c.value + d.value,
                    _probe: Probe::new(),
                });
                let value = black_box(&s).value;
                self.asyncs.push(s);
                value
            }
            "payload" => {
                let s = self.payload.get_or_insert_with(|| {
                    Arc::new(Payload {
                        bytes: vec![0x5a; PAYLOAD_BYTES.load(Ordering::Relaxed)],
                        _probe: Probe::new(),
                    })
                });
                assert_eq!(s.bytes.first(), Some(&0x5a));
                assert_eq!(s.bytes.last(), Some(&0x5a));
                black_box(s.bytes.len())
            }
            "sparse" => {
                let s = self.sparse.get_or_insert_with(|| {
                    Arc::new(Sparse {
                        leaf: OnceLock::new(),
                        _probe: Probe::new(),
                    })
                });
                black_box(s)
                    .leaf
                    .get_or_init(|| {
                        Arc::new(SparseLeaf {
                            value: 17,
                            _probe: Probe::new(),
                        })
                    })
                    .value
            }
            _ => panic!("scenario"),
        }
    }
    pub async fn close(mut self) {
        if let Some(s) = self.sparse.take() {
            // Consumer then leaf, with the same asynchronous cleanup work as DI.
            tokio::task::yield_now().await;
            CLEANED.fetch_add(1, Ordering::Relaxed);
            let s = Arc::try_unwrap(s).ok().unwrap();
            let leaf = s.leaf.into_inner().unwrap();
            drop(s._probe);
            tokio::task::yield_now().await;
            CLEANED.fetch_add(1, Ordering::Relaxed);
            drop(leaf);
        }
        while self.mixed.pop().is_some() {}
        while self.lazy.pop().is_some() {}
        while self.asyncs.pop().is_some() {}
        drop(self);
    }
}
