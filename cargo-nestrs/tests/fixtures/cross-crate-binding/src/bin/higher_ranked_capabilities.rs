//! Higher-ranked supertraits stay bound across private producer capabilities.

use contracts::{BorrowedPort, CountPort, TextPort, UnspecifiedBorrowedPort};
use fallback_provider as _;
use nestrs_core::ServiceProvider;
use primary_provider as _;

#[nestrs::injectable]
struct Reader {
    #[inject]
    text: dyn TextPort,
    #[inject]
    count: dyn CountPort<Item = usize>,
    #[inject]
    borrowed: dyn BorrowedPort,
    #[inject]
    incompatible: Option<dyn UnspecifiedBorrowedPort<Item = &'static str>>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let text = provider
        .get_required_service::<dyn TextPort>()
        .await
        .unwrap();
    let count = provider
        .get_required_service::<dyn CountPort<Item = usize> + Send + Sync>()
        .await
        .unwrap();
    let borrowed = provider
        .get_required_service::<dyn BorrowedPort>()
        .await
        .unwrap();
    assert_eq!(text.text(), "higher-ranked view");
    assert_eq!(count.count(), text.text().len());
    assert_eq!(borrowed.borrowed(), text.text());
    assert_eq!(text.identity(), count.identity());
    assert_eq!(text.identity(), borrowed.identity());
    let reader = provider.get_required_service::<Reader>().await.unwrap();
    assert_eq!(reader.text.identity(), text.identity());
    assert_eq!(reader.count.identity(), text.identity());
    assert_eq!(reader.borrowed.identity(), text.identity());
    assert!(reader.incompatible.is_none());
    assert!(
        provider
            .get_service::<dyn UnspecifiedBorrowedPort<Item = &'static str>>()
            .await
            .unwrap()
            .is_none()
    );
    provider.dispose_async().await.unwrap();
    println!(
        "cross-crate higher-ranked traits: private provider, shared identity and bound associated types passed"
    );
}
