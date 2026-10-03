//! 没有直接 core 依赖的 binary 仍支持真实类型驱动的蓝图生成与计划编译。
use query_library::{Repository, ServiceProvider};
struct OnlyApplication;
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(query_library::constructions(), 0);
    let _ = provider
        .get_required_service::<Repository<OnlyApplication>>()
        .await
        .unwrap();
    assert_eq!(query_library::constructions(), 1);
    provider.dispose_async().await.unwrap();
}
