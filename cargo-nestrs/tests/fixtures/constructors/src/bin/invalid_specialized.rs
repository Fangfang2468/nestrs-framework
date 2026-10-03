use std::marker::PhantomData;

#[nestrs::injectable]
struct Service<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
}
impl Service<u32> {
    #[nestrs::constructor]
    fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}
fn main() {}
