use nestrs as declarations;
use declarations::{factory as build_service, injectable as component};

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
    let provider = nestrs_core::ServiceProvider::build().await.unwrap();
    let consumer = nestrs_core::get_required_service!(provider, Consumer).await.unwrap();
    let database = nestrs_core::get_required_service!(provider, Database).await.unwrap();
    let _: &Configuration = nestrs_core::get_required_service!(provider, Configuration).await.unwrap();
    assert_eq!(consumer.number, 7);
    assert!(std::ptr::eq(&*consumer.database, database));
    provider.dispose_async().await.unwrap();
}
