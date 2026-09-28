#![allow(dead_code)]

use nestrs::{factory, injectable};
use nestrs_core::{ServiceProvider, get_required_service};

mod external;
use external::External;

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

trait Port: Send + Sync {
    fn label(&self) -> &'static str;
}

#[injectable]
struct Repository {
    #[value(GENERATED_COUNT)]
    count: usize,
}

impl Port for Repository {
    fn label(&self) -> &'static str {
        env!("NESTRS_FIXTURE_LABEL")
    }
}

#[injectable]
struct Service {
    #[inject]
    port: dyn Port,
    #[inject]
    external: External,
}

impl Service {
    fn describe(&self) -> &'static str {
        self.port.label()
    }
}

struct Client(&'static str);

#[factory]
async fn create_client(port: dyn Port) -> Client {
    tokio::task::yield_now().await;
    Client(port.label())
}

macro_rules! generated_provider {
    () => {
        #[nestrs::injectable]
        struct MacroValue {
            #[nestrs::value(5)]
            number: usize,
        }
    };
}
generated_provider!();

#[cfg(feature = "alternate")]
fn feature_value() -> usize {
    2
}

#[cfg(not(feature = "alternate"))]
fn feature_value() -> usize {
    1
}

#[cfg(nestrs_fixture_generated)]
fn generated_value() -> usize {
    GENERATED_COUNT
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let service = get_required_service!(provider, Service).await.unwrap();
    assert_eq!(service.describe(), "generated-by-build-script");
    assert_eq!(service.external.number, 23);
    assert_eq!(generated_value(), 17);
    assert!(feature_value() > 0);
    let _ = get_required_service!(provider, Client).await.unwrap();
    let _ = get_required_service!(provider, MacroValue).await.unwrap();
    provider.dispose_async().await.unwrap();
}
