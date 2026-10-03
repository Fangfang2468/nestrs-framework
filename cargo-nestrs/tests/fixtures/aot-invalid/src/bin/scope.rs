//! 用独立二进制验证非法生命周期，避免污染其他回归测试的编译器注册清单。
use nestrs::injectable;
use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);

fn count_construction() -> usize {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1
}

#[injectable(lifetime = Scoped)]
struct RequestSession {
    #[value(count_construction())]
    _id: usize,
}

// 请求结束时 Session 应关闭，因此整个应用共享的缓存不能持有它。
#[injectable(lifetime = Singleton)]
struct ApplicationCache {
    #[inject]
    _session: RequestSession,
    #[value(count_construction())]
    _id: usize,
}

fn main() {}
