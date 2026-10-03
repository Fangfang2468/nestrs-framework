use std::{
    alloc::{GlobalAlloc, Layout, System},
    hint::black_box,
    sync::atomic::{AtomicUsize, Ordering},
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
// SAFETY: every operation delegates the unchanged pointer/layout contract to System;
// counters use atomics and never allocate, touch payload memory, or change returned pointers.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller supplied the GlobalAlloc layout preconditions.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            acquired(layout.size());
        }
        ptr
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: identical delegation preserves alloc_zeroed layout and initialization guarantees.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            acquired(layout.size());
        }
        ptr
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: pointer and matching original layout are forwarded without modification.
        unsafe {
            System.dealloc(ptr, layout);
        }
        DEALLOCS.fetch_add(1, Ordering::Relaxed);
        released(layout.size());
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: caller guarantees a live System allocation, its layout, and valid new size.
        let new_ptr = unsafe { System.realloc(ptr, layout, size) };
        if !new_ptr.is_null() {
            REALLOCS.fetch_add(1, Ordering::Relaxed);
            // Cumulative allocation/release counters retain their replacement convention.
            // LIVE changes once: a remove-then-add pair would undercount concurrent peaks.
            FREED.fetch_add(layout.size(), Ordering::Relaxed);
            ALLOCATED.fetch_add(size, Ordering::Relaxed);
            let live = if size >= layout.size() {
                let growth = size - layout.size();
                LIVE.fetch_add(growth, Ordering::Relaxed) + growth
            } else {
                let shrink = layout.size() - size;
                LIVE.fetch_sub(shrink, Ordering::Relaxed) - shrink
            };
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        new_ptr
    }
}
#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

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
fn allocator_self_test() {
    let layout = Layout::from_size_align(24, 8).unwrap();
    let other = Layout::from_size_align(16, 8).unwrap();
    let start = Snapshot::now();
    // SAFETY: layouts are nonzero and valid; each allocation is null-checked before access,
    // realloc uses the original layout, and each successful allocation is released once.
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
static STATUS_FILE: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> =
    std::sync::OnceLock::new();
fn proc_memory_kib() -> (usize, usize) {
    use std::io::{Read, Seek};
    let mut file = STATUS_FILE
        .get_or_init(|| std::sync::Mutex::new(std::fs::File::open("/proc/self/status").unwrap()))
        .lock()
        .unwrap();
    file.rewind().unwrap();
    let mut buffer = [0u8; 16 * 1024];
    let mut length = 0;
    while length < buffer.len() {
        let count = file.read(&mut buffer[length..]).unwrap();
        if count == 0 {
            break;
        }
        length += count;
    }
    if length == buffer.len() {
        let mut extra = [0u8; 1];
        assert_eq!(
            file.read(&mut extra).unwrap(),
            0,
            "/proc/self/status exceeds measurement buffer"
        );
    }
    let status = std::str::from_utf8(&buffer[..length]).unwrap();
    let field = |name: &str| -> usize {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    (field("VmRSS:"), field("VmHWM:"))
}
