#![allow(unused_parens, unused_braces)]
use std::marker::PhantomData;

#[nestrs::injectable]
struct First;
#[nestrs::injectable]
struct Second;
#[nestrs::injectable]
struct Service<A: Send + Sync + 'static, B: Send + Sync + 'static, const N: usize> {
    first: A,
    second: B,
    marker: PhantomData<[u8; N]>,
}
impl<Left: Send + Sync + 'static, Right: Send + Sync + 'static, const M: usize>
    Service<(Right), Left, { M }>
{
    #[nestrs::constructor]
    fn new(first: Right, second: Left) -> Self {
        Self {
            first,
            second,
            marker: PhantomData,
        }
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let root = nestrs_core::ServiceProvider::build().await.unwrap();
    let value = root
        .get_required_service::<Service<First, Second, 3>>()
        .await
        .unwrap();
    assert!(std::ptr::eq(
        &*value.first,
        root.get_required_service::<First>().await.unwrap()
    ));
    assert!(std::ptr::eq(
        &*value.second,
        root.get_required_service::<Second>().await.unwrap()
    ));
    root.dispose_async().await.unwrap();
    println!("constructor generic parameter identities passed");
}
