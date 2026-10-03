use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use provider_services::{
    Cache, DeferredClass, DeferredConsumer, Port, counts, parameter_count, reset,
};

struct Customer;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    for mode in [InitializationMode::Lazy, InitializationMode::Eager] {
        reset();
        let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
            initialization: mode,
            ..Default::default()
        })
        .await
        .unwrap();
        assert_eq!(counts(), (0, 1, 1), "上游策略必须覆盖最终入口的全局默认");
        assert_eq!(
            provider
                .get_required_service::<Cache<Customer>>()
                .await
                .unwrap()
                .id,
            0
        );
        assert_eq!(
            provider
                .get_required_service::<DeferredClass>()
                .await
                .unwrap()
                .id,
            0
        );
        assert_eq!(
            provider
                .get_required_service::<dyn Port>()
                .await
                .unwrap()
                .id(),
            1
        );
        assert_eq!(counts(), (2, 1, 1));
        let deferred = provider
            .get_required_service::<DeferredConsumer>()
            .await
            .unwrap();
        assert_eq!(
            parameter_count(),
            0,
            "上游 factory 的延迟参数不能成为构造前提"
        );
        assert_eq!(deferred.target_id().await, 91);
        assert_eq!(deferred.target_id().await, 91);
        assert_eq!(parameter_count(), 1);
        provider.dispose_async().await.unwrap();
    }
    println!("cross-crate provider lazy: eager/lazy overrides and closed generic policy preserved");
}
