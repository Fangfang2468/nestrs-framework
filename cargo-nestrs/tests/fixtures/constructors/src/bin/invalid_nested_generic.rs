use std::marker::PhantomData;

#[nestrs::injectable]
struct Service<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
}
impl<T: Send + Sync + 'static> Service<Vec<T>> {
    #[nestrs::constructor]
    fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}
fn main() {}
