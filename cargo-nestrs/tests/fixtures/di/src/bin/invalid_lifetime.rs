//! 用独立二进制验证非法生命周期，避免污染其他回归测试的 linkme 注册集合。
use nestrs::injectable;
use std::{
    process::ExitCode,
    sync::atomic::{AtomicUsize, Ordering},
};

use nestrs_core::ServiceProvider;

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

#[tokio::main]
async fn main() -> ExitCode {
    println!("故意声明错误依赖：Singleton ApplicationCache -> Scoped RequestSession");
    // JoinHandle 捕获真实 build panic，仅为在其后打印构造计数；仍以非零状态退出。
    let result = tokio::spawn(ServiceProvider::build()).await;
    let constructions = CONSTRUCTIONS.load(Ordering::SeqCst);
    match result {
        Err(error) if error.is_panic() && constructions == 0 => {
            println!("图验证已在服务实例化前拒绝依赖，构造次数 = 0");
            ExitCode::FAILURE
        }
        Ok(Ok(provider)) => {
            let _ = provider.dispose_async().await;
            eprintln!("错误：非法依赖图竟然通过验证，构造次数 = {constructions}");
            ExitCode::from(2)
        }
        other => {
            // ServiceProvider 不实现 Debug，避免把整个 Result 作为调试值打印。
            drop(other);
            eprintln!("发生非预期结果，构造次数 = {constructions}");
            ExitCode::from(2)
        }
    }
}
