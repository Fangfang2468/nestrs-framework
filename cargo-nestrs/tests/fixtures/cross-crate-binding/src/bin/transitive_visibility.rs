//! 未重导出的传递 trait 不污染生成源码，合法重导出的业务能力仍保留。

use contracts::{ExposedMarker, PublicCapability};
use nestrs::injectable;
use nestrs_core::ServiceProvider;

#[injectable]
struct Service {
    #[value(42)]
    value: usize,
}

impl ExposedMarker for Service {
    fn value(&self) -> usize {
        self.value
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let service = provider.get_required_service::<Service>().await.unwrap();
    let exposed = provider
        .get_required_service::<dyn PublicCapability>()
        .await
        .unwrap();
    assert_eq!(exposed.exposed_value(), 42);
    assert_eq!(exposed.identity(), service as *const Service as usize);
    provider.dispose_async().await.unwrap();
    println!(
        "cross-crate visibility: inaccessible transitive capabilities skipped and reexported blanket trait preserved"
    );
}
