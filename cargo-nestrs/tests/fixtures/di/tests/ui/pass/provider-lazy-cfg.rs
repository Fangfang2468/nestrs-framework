use nestrs::{factory, injectable, lazy};
use std::sync::atomic::{AtomicUsize, Ordering};
static CREATED: AtomicUsize = AtomicUsize::new(0);

#[cfg(any())]
#[lazy("disabled invalid provider")]
struct Removed;

#[cfg_attr(all(), lazy(false))]
#[injectable]
#[cfg_attr(any(), lazy(true))]
struct Configured {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}

struct Client;
#[factory]
#[cfg_attr(all(), nestrs::lazy(false))]
fn client() -> Client {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Client
}

#[lazy()]
#[injectable]
struct EmptyOuter;
#[injectable]
#[lazy()]
struct EmptyInner;

#[tokio::main]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build(None).await.unwrap();
    assert_eq!(CREATED.load(Ordering::SeqCst), 2);
    provider.dispose_async().await.unwrap();
}
