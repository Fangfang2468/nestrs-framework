//! Identical workload driver for before/after DI and explicit manual composition.
#![allow(dead_code)]
include!("metrics.rs");
use std::{sync::Arc, time::Instant};
#[cfg(not(feature = "manual"))]
mod di;
#[cfg(not(feature = "manual"))]
use di as backend;
#[cfg(feature = "manual")]
mod manual;
#[cfg(feature = "manual")]
use manual as backend;

static CREATED: AtomicUsize = AtomicUsize::new(0);
static DROPPED: AtomicUsize = AtomicUsize::new(0);
static CLEANED: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_CONCURRENCY: AtomicUsize = AtomicUsize::new(0);
static PAYLOAD_BYTES: AtomicUsize = AtomicUsize::new(0);
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

fn sample(
    stage: &str,
    wave: usize,
    baseline: Snapshot,
    query_ns: u128,
    close_ns: u128,
    checksum: usize,
) {
    let now = Snapshot::now();
    let peak = PEAK.load(Ordering::Relaxed);
    let (rss, hwm) = proc_memory_kib();
    println!(
        "{{\"stage\":\"{}\",\"wave\":{},\"concurrency\":{},\"allocs\":{},\"reallocs\":{},\"deallocs\":{},\"allocated_bytes\":{},\"freed_bytes\":{},\"live_bytes\":{},\"peak_live_bytes\":{},\"rss_kib\":{},\"vm_hwm_kib\":{},\"created\":{},\"dropped\":{},\"cleaned\":{},\"query_ns\":{},\"close_ns\":{},\"checksum\":{}}}",
        stage,
        wave,
        ACTIVE_CONCURRENCY.load(Ordering::Relaxed),
        now.allocs - baseline.allocs,
        now.reallocs - baseline.reallocs,
        now.deallocs - baseline.deallocs,
        now.allocated - baseline.allocated,
        now.freed - baseline.freed,
        now.live,
        peak,
        rss,
        hwm,
        CREATED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed),
        CLEANED.load(Ordering::Relaxed),
        query_ns,
        close_ns,
        checksum
    );
}

async fn run(
    scenario: &'static str,
    waves: usize,
    concurrency: usize,
    per_scope: usize,
    hold_ms: u64,
    burst: usize,
) {
    use tokio::sync::{Barrier, Semaphore};
    let root = Arc::new(backend::Root::new().await);
    println!(
        "{{\"backend\":\"{}\",\"scenario\":\"{}\",\"waves\":{},\"concurrency\":{},\"queries_per_scope\":{},\"hold_ms\":{},\"payload_bytes\":{},\"workers\":2,\"large_graph\":{}}}",
        if cfg!(feature = "manual") {
            "manual"
        } else {
            "nestrs"
        },
        scenario,
        waves,
        concurrency,
        per_scope,
        hold_ms,
        PAYLOAD_BYTES.load(Ordering::Relaxed),
        cfg!(feature = "large-graph")
    );
    let _ = proc_memory_kib();
    let expected = match scenario {
        "mixed" | "scoped" => 119,
        "lazy4" | "async4" => 68,
        "payload" => PAYLOAD_BYTES.load(Ordering::Relaxed),
        "sparse" => 17,
        _ => panic!("scenario"),
    };
    let per_scope_instances = match scenario {
        "scoped" | "payload" => 1,
        "sparse" => 2,
        _ => per_scope,
    };
    let mut total = 0;
    let mut expected_instances = 0;
    for wave in 0..waves + 3 {
        let warm = wave < 3;
        let concurrency = if wave == 3 && burst > 0 {
            burst
        } else {
            concurrency
        };
        ACTIVE_CONCURRENCY.store(concurrency, Ordering::Relaxed);
        let baseline = Snapshot::now();
        PEAK.store(baseline.live, Ordering::Relaxed);
        let started = Instant::now();
        let ready = Arc::new(Barrier::new(concurrency + 1));
        let release = Arc::new(Semaphore::new(0));
        let mut jobs = tokio::task::JoinSet::new();
        for _ in 0..concurrency {
            let root = root.clone();
            let ready = ready.clone();
            let release = release.clone();
            jobs.spawn(async move {
                let mut scope = root.scope();
                let mut checksum = 0;
                for _ in 0..per_scope {
                    checksum += scope.query(scenario).await;
                }
                ready.wait().await;
                release.acquire().await.unwrap().forget();
                scope.close().await;
                checksum
            });
        }
        tokio::time::timeout(std::time::Duration::from_secs(60), ready.wait())
            .await
            .unwrap();
        let query_ns = started.elapsed().as_nanos();
        if !warm {
            sample("held", wave - 3, baseline, query_ns, 0, 0);
        }
        if hold_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(hold_ms)).await;
        }
        let close_start = Instant::now();
        release.add_permits(concurrency);
        let mut checksum = 0;
        while let Some(result) = jobs.join_next().await {
            checksum += result.unwrap();
        }
        let close_ns = close_start.elapsed().as_nanos();
        assert_eq!(checksum, expected * concurrency * per_scope);
        drop(jobs);
        drop(ready);
        drop(release);
        root.settle().await;
        expected_instances += concurrency * per_scope_instances;
        assert_eq!(CREATED.load(Ordering::Relaxed), expected_instances);
        assert_eq!(DROPPED.load(Ordering::Relaxed), expected_instances);
        if scenario == "sparse" {
            assert_eq!(CLEANED.load(Ordering::Relaxed), expected_instances);
        }
        if !warm {
            total += checksum;
            sample("closed", wave - 3, baseline, query_ns, close_ns, checksum);
        }
    }
    let baseline = Snapshot::now();
    let close_start = Instant::now();
    let root = Arc::try_unwrap(root)
        .ok()
        .expect("all requests released root");
    root.close().await;
    let close_ns = close_start.elapsed().as_nanos();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert_eq!(
        CREATED.load(Ordering::Relaxed),
        DROPPED.load(Ordering::Relaxed)
    );
    sample("root_closed", waves, baseline, 0, close_ns, total);
}

fn main() {
    allocator_self_test();
    let args: Vec<_> = std::env::args().skip(1).collect();
    let scenario = match args.first().map(String::as_str).unwrap_or("mixed") {
        "mixed" => "mixed",
        "scoped" => "scoped",
        "lazy4" => "lazy4",
        "async4" => "async4",
        "payload" => "payload",
        "sparse" => "sparse",
        _ => panic!("scenario"),
    };
    let waves = args.get(1).map_or(32, |s| s.parse().unwrap());
    let concurrency = args.get(2).map_or(64, |s| s.parse().unwrap());
    let queries = args.get(3).map_or(8, |s| s.parse().unwrap());
    PAYLOAD_BYTES.store(
        args.get(4).map_or(65536, |s| s.parse().unwrap()),
        Ordering::Relaxed,
    );
    let hold = args.get(5).map_or(0, |s| s.parse().unwrap());
    assert!(waves > 0 && concurrency > 0 && queries > 0);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let burst = args.get(6).map_or(0, |s| s.parse().unwrap());
    runtime.block_on(run(scenario, waves, concurrency, queries, hold, burst));
}
