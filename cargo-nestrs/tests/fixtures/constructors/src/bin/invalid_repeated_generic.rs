use std::marker::PhantomData;

#[nestrs::injectable]
struct Service<T: Send + Sync + 'static, U: Send + Sync + 'static> {
    marker: PhantomData<(T, U)>,
}
impl<T: Send + Sync + 'static> Service<T, T> {
    #[nestrs::constructor]
    fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}
fn main() {}
