//! A migration case: an existing explicit pair must not be generated a second time.

use nestrs::{bind, injectable};
use nestrs_core::ServiceProvider;

#[path = "../automatic_assertions.rs"]
mod automatic_assertions;

trait Port: Send + Sync {
    fn identity(&self) -> usize;
}

#[injectable]
struct Service {
    #[value(42)]
    _nonzero_size: usize,
}

#[bind]
impl Port for Service {
    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

#[tokio::main]
async fn main() {
    assert_eq!(automatic_assertions::explicit_count(), 1);
    automatic_assertions::assert_count::<dyn Port>(0);
    let provider = ServiceProvider::build(None).await.unwrap();
    let concrete = provider.get_required_service::<Service>().await.unwrap();
    let interface = provider.get_required_service::<dyn Port>().await.unwrap();
    assert_eq!(interface.identity(), concrete as *const Service as usize);
    provider.dispose_async().await.unwrap();
    println!("auto-binding explicit pair: one binding and one shared singleton");
}
