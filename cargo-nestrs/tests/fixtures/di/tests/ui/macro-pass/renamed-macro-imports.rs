use declarations::{factory as build_service, injectable as component};
use nestrs as declarations;

#[component]
struct Database;

#[component]
struct Consumer {
    #[inject]
    database: Database,
    #[value(7)]
    number: usize,
}

struct Configuration;

#[build_service]
fn configuration() -> Configuration {
    Configuration
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build(None).await.unwrap();
    let consumer = provider.get_required_service::<Consumer>().await.unwrap();
    let database = provider.get_required_service::<Database>().await.unwrap();
    let _: &Configuration = provider
        .get_required_service::<Configuration>()
        .await
        .unwrap();
    assert_eq!(consumer.number, 7);
    assert!(std::ptr::eq(&*consumer.database, database));
    provider.dispose_async().await.unwrap();
}
