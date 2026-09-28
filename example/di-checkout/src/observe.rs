//! 用时间和实例编号观察初始化、共享及关闭，不读取容器内部状态。

use std::{
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

static START: OnceLock<Instant> = OnceLock::new();
static INSTANCE_ID: AtomicUsize = AtomicUsize::new(1);

pub fn event(message: impl AsRef<str>) {
    let elapsed = START.get_or_init(Instant::now).elapsed().as_millis();
    println!("[+{elapsed:>4}ms] {}", message.as_ref());
}

/// 编号仅用于解释实例身份，业务订单号由业务层单独生成。
pub fn created(name: &str) -> usize {
    let id = INSTANCE_ID.fetch_add(1, Ordering::Relaxed);
    event(format!("[构造] {name} #{id}"));
    id
}
