use std::marker::PhantomData;

#[nestrs::injectable]
struct Service<const N: usize> {
    marker: PhantomData<[u8; N]>,
}
impl Service<3> {
    #[nestrs::constructor]
    fn new() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}
fn main() {}
