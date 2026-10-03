//! Same business source is built by each version's own compiler toolchain.
//! Allocator records requested bytes, not allocator metadata or resident pages.
use nestrs::{factory, injectable};
use nestrs_core::{ServiceProvider, ServiceProviderRef};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
    time::Instant,
};

struct CountingAllocator;
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static REALLOCS: AtomicUsize = AtomicUsize::new(0);
static DEALLOCS: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static FREED: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn acquired(bytes: usize) {
    ALLOCATED.fetch_add(bytes, Ordering::Relaxed);
    let live = LIVE.fetch_add(bytes, Ordering::Relaxed) + bytes;
    PEAK.fetch_max(live, Ordering::Relaxed);
}
fn released(bytes: usize) {
    FREED.fetch_add(bytes, Ordering::Relaxed);
    LIVE.fetch_sub(bytes, Ordering::Relaxed);
}
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            acquired(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            acquired(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(ptr, layout);
        }
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        released(layout.size());
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, size) };
        if !new_ptr.is_null() {
            REALLOCS.fetch_add(1, Ordering::Relaxed);
            // Count replacement requested size and old request release; failed realloc preserves old allocation.
            released(layout.size());
            acquired(size);
        }
        new_ptr
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

static CREATED: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);
struct Probe;
impl Probe {
    fn new() -> Self {
        CREATED.fetch_add(1, Ordering::Relaxed);
        Self
    }
}
impl Drop for Probe {
    fn drop(&mut self) {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

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

#[injectable(lifetime = Transient)]
struct Empty {
    #[value(Probe::new())]
    _probe: Probe,
}

#[injectable(lifetime = Transient)]
struct Concrete1 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: Base,
}

#[injectable(lifetime = Transient)]
struct Concrete4 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: Base,
    #[inject]
    d1: Base,
    #[inject]
    d2: Base,
    #[inject]
    d3: Base,
}

#[injectable(lifetime = Transient)]
struct Trait1 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: dyn Value,
}

#[injectable(lifetime = Transient)]
struct Trait4 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: dyn Value,
    #[inject]
    d1: dyn Value,
    #[inject]
    d2: dyn Value,
    #[inject]
    d3: dyn Value,
}

#[injectable(lifetime = Transient)]
struct Optional1 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: Option<Base>,
}

#[injectable(lifetime = Transient)]
struct Optional4 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: Option<Base>,
    #[inject]
    d1: Option<Base>,
    #[inject]
    d2: Option<Base>,
    #[inject]
    d3: Option<Base>,
}

#[injectable(lifetime = Transient)]
struct Absent1 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: Option<Missing>,
}

#[injectable(lifetime = Transient)]
struct Absent4 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    d0: Option<Missing>,
    #[inject]
    d1: Option<Missing>,
    #[inject]
    d2: Option<Missing>,
    #[inject]
    d3: Option<Missing>,
}

#[injectable(lifetime = Transient)]
struct Lazy1 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    #[lazy]
    d0: LazyBase,
}

#[injectable(lifetime = Transient)]
struct Lazy4 {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    #[lazy]
    d0: LazyBase,
    #[inject]
    #[lazy]
    d1: LazyBase,
    #[inject]
    #[lazy]
    d2: LazyBase,
    #[inject]
    #[lazy]
    d3: LazyBase,
}

struct Sync0 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime = Transient)]
fn make_sync0() -> Sync0 {
    Sync0 {
        value: 1,
        _probe: Probe::new(),
    }
}

struct Sync1 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime = Transient)]
fn make_sync1(d0: Base) -> Sync1 {
    Sync1 {
        value: d0.value,
        _probe: Probe::new(),
    }
}

struct Sync4 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime = Transient)]
fn make_sync4(d0: Base, d1: Base, d2: Base, d3: Base) -> Sync4 {
    Sync4 {
        value: d0.value + d1.value + d2.value + d3.value,
        _probe: Probe::new(),
    }
}

struct Async0 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime = Transient)]
async fn make_async0() -> Async0 {
    tokio::task::yield_now().await;
    Async0 {
        value: 1,
        _probe: Probe::new(),
    }
}

struct Async1 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime = Transient)]
async fn make_async1(d0: Base) -> Async1 {
    tokio::task::yield_now().await;
    Async1 {
        value: d0.value,
        _probe: Probe::new(),
    }
}

struct Async4 {
    value: usize,
    _probe: Probe,
}
#[factory(lifetime = Transient)]
async fn make_async4(d0: Base, d1: Base, d2: Base, d3: Base) -> Async4 {
    tokio::task::yield_now().await;
    Async4 {
        value: d0.value + d1.value + d2.value + d3.value,
        _probe: Probe::new(),
    }
}

#[injectable(lifetime = Transient)]
struct Mixed {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    concrete1: Base,
    #[inject]
    concrete2: Base,
    #[inject]
    interface1: dyn Value,
    #[inject]
    interface2: dyn Value,
    #[inject]
    optional: Option<Base>,
    #[inject]
    absent: Option<Missing>,
    #[inject]
    #[lazy]
    lazy1: LazyBase,
    #[inject]
    #[lazy]
    lazy2: LazyBase,
}

#[injectable(lifetime = Scoped)]
struct Scoped {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    concrete1: Base,
    #[inject]
    concrete2: Base,
    #[inject]
    interface1: dyn Value,
    #[inject]
    interface2: dyn Value,
    #[inject]
    optional: Option<Base>,
    #[inject]
    absent: Option<Missing>,
    #[inject]
    #[lazy]
    lazy1: LazyBase,
    #[inject]
    #[lazy]
    lazy2: LazyBase,
}

#[injectable(lifetime = Singleton)]
struct Singleton {
    #[value(Probe::new())]
    _probe: Probe,
    #[inject]
    concrete1: Base,
    #[inject]
    concrete2: Base,
    #[inject]
    interface1: dyn Value,
    #[inject]
    interface2: dyn Value,
    #[inject]
    optional: Option<Base>,
    #[inject]
    absent: Option<Missing>,
    #[inject]
    #[lazy]
    lazy1: LazyBase,
    #[inject]
    #[lazy]
    lazy2: LazyBase,
}

async fn query(provider: ServiceProviderRef<'_>, scenario: &str) -> usize {
    match scenario {
        "empty" => {
            let service = black_box(provider.get_required_service::<Empty>().await.unwrap());
            black_box(&*service);
            1
        }
        "concrete1" => {
            let service = black_box(provider.get_required_service::<Concrete1>().await.unwrap());
            service.d0.value
        }
        "concrete4" => {
            let service = black_box(provider.get_required_service::<Concrete4>().await.unwrap());
            service.d0.value + service.d1.value + service.d2.value + service.d3.value
        }
        "trait1" => {
            let service = black_box(provider.get_required_service::<Trait1>().await.unwrap());
            service.d0.value()
        }
        "trait4" => {
            let service = black_box(provider.get_required_service::<Trait4>().await.unwrap());
            service.d0.value() + service.d1.value() + service.d2.value() + service.d3.value()
        }
        "optional1" => {
            let service = black_box(provider.get_required_service::<Optional1>().await.unwrap());
            service.d0.as_ref().unwrap().value
        }
        "optional4" => {
            let service = black_box(provider.get_required_service::<Optional4>().await.unwrap());
            service.d0.as_ref().unwrap().value
                + service.d1.as_ref().unwrap().value
                + service.d2.as_ref().unwrap().value
                + service.d3.as_ref().unwrap().value
        }
        "absent1" => {
            let service = black_box(provider.get_required_service::<Absent1>().await.unwrap());
            usize::from(service.d0.is_some())
        }
        "absent4" => {
            let service = black_box(provider.get_required_service::<Absent4>().await.unwrap());
            usize::from(service.d0.is_some())
                + usize::from(service.d1.is_some())
                + usize::from(service.d2.is_some())
                + usize::from(service.d3.is_some())
        }
        "lazy1" => {
            let service = black_box(provider.get_required_service::<Lazy1>().await.unwrap());
            service.d0.get().await.unwrap().value
        }
        "lazy-unused1" => {
            let service = black_box(provider.get_required_service::<Lazy1>().await.unwrap());
            black_box(&*service);
            1
        }
        "lazy4" => {
            let service = black_box(provider.get_required_service::<Lazy4>().await.unwrap());
            service.d0.get().await.unwrap().value
                + service.d1.get().await.unwrap().value
                + service.d2.get().await.unwrap().value
                + service.d3.get().await.unwrap().value
        }
        "lazy-unused4" => {
            let service = black_box(provider.get_required_service::<Lazy4>().await.unwrap());
            black_box(&*service);
            4
        }
        "sync0" => {
            let service = black_box(provider.get_required_service::<Sync0>().await.unwrap());
            service.value
        }
        "sync1" => {
            let service = black_box(provider.get_required_service::<Sync1>().await.unwrap());
            service.value
        }
        "sync4" => {
            let service = black_box(provider.get_required_service::<Sync4>().await.unwrap());
            service.value
        }
        "async0" => {
            let service = black_box(provider.get_required_service::<Async0>().await.unwrap());
            service.value
        }
        "async1" => {
            let service = black_box(provider.get_required_service::<Async1>().await.unwrap());
            service.value
        }
        "async4" => {
            let service = black_box(provider.get_required_service::<Async4>().await.unwrap());
            service.value
        }
        "mixed" => {
            let service = black_box(provider.get_required_service::<Mixed>().await.unwrap());
            service.concrete1.value
                + service.concrete2.value
                + service.interface1.value()
                + service.interface2.value()
                + service.optional.as_ref().unwrap().value
                + usize::from(service.absent.is_some())
                + service.lazy1.get().await.unwrap().value
                + service.lazy2.get().await.unwrap().value
        }
        "scoped" => {
            let service = black_box(provider.get_required_service::<Scoped>().await.unwrap());
            service.concrete1.value
                + service.concrete2.value
                + service.interface1.value()
                + service.interface2.value()
                + service.optional.as_ref().unwrap().value
                + usize::from(service.absent.is_some())
                + service.lazy1.get().await.unwrap().value
                + service.lazy2.get().await.unwrap().value
        }
        "singleton" => {
            let service = black_box(provider.get_required_service::<Singleton>().await.unwrap());
            service.concrete1.value
                + service.concrete2.value
                + service.interface1.value()
                + service.interface2.value()
                + service.optional.as_ref().unwrap().value
                + usize::from(service.absent.is_some())
                + service.lazy1.get().await.unwrap().value
                + service.lazy2.get().await.unwrap().value
        }
        _ => panic!("unknown scenario: {scenario}"),
    }
}

#[derive(Clone, Copy, Default)]
struct Snapshot {
    allocs: usize,
    reallocs: usize,
    deallocs: usize,
    allocated: usize,
    freed: usize,
    live: usize,
}
impl Snapshot {
    fn now() -> Self {
        Self {
            allocs: ALLOCS.load(Ordering::Relaxed),
            reallocs: REALLOCS.load(Ordering::Relaxed),
            deallocs: DEALLOCS.load(Ordering::Relaxed),
            allocated: ALLOCATED.load(Ordering::Relaxed),
            freed: FREED.load(Ordering::Relaxed),
            live: LIVE.load(Ordering::Relaxed),
        }
    }
}
#[derive(Clone, Copy, Default)]
struct Checkpoint {
    live: usize,
    rss_kib: usize,
}
#[derive(Default)]
struct Measurement {
    allocs: usize,
    reallocs: usize,
    deallocs: usize,
    allocated: usize,
    freed: usize,
    peak_extra: usize,
    peak_live: usize,
    ns: u128,
    checksum: usize,
}
fn process_memory_kib(field: &str) -> usize {
    std::fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find(|line| line.starts_with(field))
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap()
}
fn rss() -> usize {
    process_memory_kib("VmRSS:")
}
async fn settle(provider: &ServiceProvider) {
    black_box(provider.get_required_service::<Base>().await.unwrap());
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
}
async fn run(scenario: &str, batches: usize, batch_size: usize) {
    assert!(batches <= 128 && batches > 0 && batch_size > 0);
    let provider = ServiceProvider::build(None).await.unwrap();
    settle(&provider).await;
    // Warm all allocations, selected routes, graph OnceLock, root singletons, and a bounded owner lifecycle.
    for _ in 0..8 {
        let scope = provider.create_scope(None).await.unwrap();
        for _ in 0..batch_size {
            black_box(query(scope.service_provider(), scenario).await);
        }
        scope.dispose_async().await.unwrap();
        settle(&provider).await;
    }
    let mut checkpoints = [Checkpoint::default(); 128];
    let rss_start = rss();
    let steady_start = Snapshot::now();
    let created_start = CREATED.load(Ordering::Relaxed);
    let dropped_start = DROPPED.load(Ordering::Relaxed);
    let mut result = Measurement::default();
    for checkpoint in checkpoints.iter_mut().take(batches) {
        let scope = provider.create_scope(None).await.unwrap();
        black_box(
            scope
                .service_provider()
                .get_required_service::<Base>()
                .await
                .unwrap(),
        );
        let start = Snapshot::now();
        PEAK.store(start.live, Ordering::Relaxed);
        let timer = Instant::now();
        for _ in 0..batch_size {
            result.checksum += black_box(query(scope.service_provider(), scenario).await);
        }
        result.ns += timer.elapsed().as_nanos();
        let end = Snapshot::now();
        result.peak_live = result.peak_live.max(PEAK.load(Ordering::Relaxed));
        result.peak_extra = result
            .peak_extra
            .max(PEAK.load(Ordering::Relaxed).saturating_sub(start.live));
        result.allocs += end.allocs - start.allocs;
        result.reallocs += end.reallocs - start.reallocs;
        result.deallocs += end.deallocs - start.deallocs;
        result.allocated += end.allocated - start.allocated;
        result.freed += end.freed - start.freed;
        scope.dispose_async().await.unwrap();
        settle(&provider).await;
        checkpoint.live = LIVE.load(Ordering::Relaxed);
        checkpoint.rss_kib = rss();
    }
    let steady_end = Snapshot::now();
    let created = CREATED.load(Ordering::Relaxed) - created_start;
    let dropped = DROPPED.load(Ordering::Relaxed) - dropped_start;
    let expected = match scenario {
        "singleton" => 0,
        "scoped" => batches,
        _ => batches * batch_size,
    };
    assert_eq!(created, expected, "incorrect number of constructors");
    assert_eq!(
        dropped, expected,
        "a measured instance survived all scope closes"
    );
    let expected_value = match scenario {
        "empty" | "sync0" | "async0" | "lazy-unused1" => 1,
        "lazy-unused4" => 4,
        "absent1" | "absent4" => 0,
        "concrete1" | "trait1" | "optional1" | "lazy1" | "sync1" | "async1" => 17,
        "concrete4" | "trait4" | "optional4" | "lazy4" | "sync4" | "async4" => 68,
        "mixed" | "scoped" | "singleton" => 119,
        _ => unreachable!(),
    };
    assert_eq!(
        result.checksum,
        expected_value * batches * batch_size,
        "incorrect business result"
    );
    let rss_end = rss();
    provider.dispose_async().await.unwrap();
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    let root_closed_live = LIVE.load(Ordering::Relaxed);
    assert_eq!(
        CREATED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        "root close left probe instances alive"
    );
    let process_vm_hwm = process_memory_kib("VmHWM:");
    // Printing occurs only after snapshots: JSON formatting does not pollute measured windows.
    print!(
        "{{\"scenario\":\"{}\",\"queries\":{},\"batches\":{},\"batch_size\":{},\"checksum\":{},\"created\":{},\"dropped\":{},\"allocation_calls\":{},\"reallocation_calls\":{},\"deallocation_calls\":{},\"allocated_bytes\":{},\"freed_bytes\":{},\"peak_live_bytes\":{},\"peak_live_extra_bytes\":{},\"steady_start_live_bytes\":{},\"steady_end_live_bytes\":{},\"root_closed_live_bytes\":{},\"rss_start_kib\":{},\"rss_end_kib\":{},\"process_vm_hwm_kib\":{},\"elapsed_ns\":{},\"checkpoints\":[",
        scenario,
        batches * batch_size,
        batches,
        batch_size,
        result.checksum,
        created,
        dropped,
        result.allocs,
        result.reallocs,
        result.deallocs,
        result.allocated,
        result.freed,
        result.peak_live,
        result.peak_extra,
        steady_start.live,
        steady_end.live,
        root_closed_live,
        rss_start,
        rss_end,
        process_vm_hwm,
        result.ns
    );
    for (i, checkpoint) in checkpoints.iter().take(batches).enumerate() {
        if i > 0 {
            print!(",");
        }
        print!(
            "{{\"live_bytes\":{},\"rss_kib\":{}}}",
            checkpoint.live, checkpoint.rss_kib
        );
    }
    println!("]}}");
}
// Runs before Tokio starts, so no other thread can mutate allocation counters.
fn allocator_self_test() {
    let layout = Layout::from_size_align(24, 8).unwrap();
    let other = Layout::from_size_align(16, 8).unwrap();
    let start = Snapshot::now();
    unsafe {
        let a = black_box(std::alloc::alloc_zeroed(layout));
        assert!(!a.is_null());
        assert_eq!(*a, 0);
        *a = 42;
        let a = black_box(std::alloc::realloc(a, layout, 48));
        assert!(!a.is_null());
        assert_eq!(*a, 42);
        let b = black_box(std::alloc::alloc(other));
        assert!(!b.is_null());
        std::alloc::dealloc(a, Layout::from_size_align(48, 8).unwrap());
        std::alloc::dealloc(b, other);
    }
    let end = Snapshot::now();
    assert_eq!(end.allocs - start.allocs, 2);
    assert_eq!(end.reallocs - start.reallocs, 1);
    assert_eq!(end.deallocs - start.deallocs, 2);
    assert_eq!(end.allocated - start.allocated, 88);
    assert_eq!(end.freed - start.freed, 88);
    assert_eq!(end.live, start.live);
}
fn main() {
    allocator_self_test();
    // Exclude argv[0]: before/after executable paths have different lengths.
    let args: Vec<_> = std::env::args().skip(1).collect();
    let scenario = args.first().expect("scenario");
    let batches = args.get(1).map_or(32, |v| v.parse().unwrap());
    let batch_size = args.get(2).map_or(32, |v| v.parse().unwrap());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(run(scenario, batches, batch_size));
}
