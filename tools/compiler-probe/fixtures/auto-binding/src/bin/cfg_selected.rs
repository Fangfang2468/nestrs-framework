//! Each target/feature analysis must describe the active compiler configuration.

use nestrs::injectable;
use nestrs_core::{__private::REFLECTED_BINDINGS, ServiceProvider};

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

trait Port: Send + Sync {
    fn selected(&self) -> &'static str;
}

#[cfg(not(feature = "alternate"))]
#[injectable]
struct DefaultService;

#[cfg(not(feature = "alternate"))]
impl Port for DefaultService {
    fn selected(&self) -> &'static str {
        "default"
    }
}

#[cfg(feature = "alternate")]
#[injectable]
struct AlternateService;

#[cfg(feature = "alternate")]
impl Port for AlternateService {
    fn selected(&self) -> &'static str {
        "alternate"
    }
}

#[tokio::main]
async fn main() {
    assert_eq!(REFLECTED_BINDINGS.len(), 0);
    automatic_assertions::assert_count::<dyn Port>(1);
    let provider = ServiceProvider::build().await.unwrap();
    let service = nestrs_core::get_required_service!(provider, dyn Port)
        .await
        .unwrap();
    assert_eq!(
        service.selected(),
        if cfg!(feature = "alternate") {
            "alternate"
        } else {
            "default"
        }
    );
    provider.dispose_async().await.unwrap();
    println!("auto-binding cfg selection: selected compiler branch only");
}
