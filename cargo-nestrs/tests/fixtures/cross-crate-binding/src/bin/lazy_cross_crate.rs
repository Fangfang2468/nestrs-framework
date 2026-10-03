//! lib 内的延迟声明、上游私有工厂和下游查询根共同进入一个冻结图。

use contracts::ConnectionPort;
use fallback_provider as _;
use nestrs_core::ServiceProvider;
use upstream_consumer::LazyConnectionConsumer;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(upstream_consumer::linked_provider_constructions(), 0);
    let provider = ServiceProvider::build().await.unwrap();
    let consumer = provider
        .get_required_service::<LazyConnectionConsumer>()
        .await
        .unwrap();
    // 消费者可以发布，但它的接口目标及 async factory 均未构造。
    assert_eq!(primary_provider::total_constructions(), 0);
    let identity = consumer.connection_identity().await.unwrap();
    assert_eq!(primary_provider::total_constructions(), 1);
    assert_eq!(consumer.connection_identity().await.unwrap(), identity);
    let connection = provider
        .get_required_service::<dyn ConnectionPort>()
        .await
        .unwrap();
    assert_eq!(connection.identity(), identity);
    provider.dispose_async().await.unwrap();
    assert_eq!(primary_provider::connection_cleanup_count(), 1);
    assert_eq!(primary_provider::connection_drop_count(), 1);
    println!("cross-crate lazy: library field shares the upstream private async factory instance");
}
