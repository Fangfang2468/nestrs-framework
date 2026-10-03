use nestrs::{factory, injectable, lazy, primary};
use std::sync::atomic::{AtomicUsize, Ordering};
static CREATED: AtomicUsize = AtomicUsize::new(0);
#[injectable]
#[lazy(false)]
#[primary]
struct Service0 {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}
#[injectable]
#[primary]
#[lazy(false)]
struct Service1 {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}
#[lazy(false)]
#[injectable]
#[primary]
struct Service2 {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}
#[lazy(false)]
#[primary]
#[injectable]
struct Service3 {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}
#[primary]
#[injectable]
#[lazy(false)]
struct Service4 {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}
#[primary]
#[lazy(false)]
#[injectable]
struct Service5 {
    #[value(CREATED.fetch_add(1, Ordering::SeqCst))]
    _id: usize,
}
struct Product0;
#[factory]
#[lazy(false)]
#[primary]
fn product_0() -> Product0 {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Product0
}
struct Product1;
#[factory]
#[primary]
#[lazy(false)]
fn product_1() -> Product1 {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Product1
}
struct Product2;
#[lazy(false)]
#[factory]
#[primary]
fn product_2() -> Product2 {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Product2
}
struct Product3;
#[lazy(false)]
#[primary]
#[factory]
fn product_3() -> Product3 {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Product3
}
struct Product4;
#[primary]
#[factory]
#[lazy(false)]
fn product_4() -> Product4 {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Product4
}
struct Product5;
#[primary]
#[lazy(false)]
#[factory]
fn product_5() -> Product5 {
    CREATED.fetch_add(1, Ordering::SeqCst);
    Product5
}
#[tokio::main]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build().await.unwrap();
    assert_eq!(CREATED.load(Ordering::SeqCst), 12);
    provider.dispose_async().await.unwrap();
}
