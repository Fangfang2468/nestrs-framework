use nestrs::injectable;
use nestrs_core::ServiceProvider;

use std::sync::atomic::{AtomicUsize, Ordering};
static DROPS: AtomicUsize = AtomicUsize::new(0);
#[injectable]
struct Value {
    #[value(42)]
    number: usize,
}
impl Drop for Value {
    fn drop(&mut self) {
        DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn service_reference_outlives_the_tokio_runtime_until_its_owner_is_dropped() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let provider = runtime.block_on(ServiceProvider::build(None)).unwrap();
    let value = runtime
        .block_on(provider.get_required_service::<Value>())
        .unwrap();
    drop(runtime);
    assert_eq!(DROPS.load(Ordering::SeqCst), 0);
    assert_eq!(value.number, 42);
    drop(provider);
    assert_eq!(DROPS.load(Ordering::SeqCst), 1);
}
