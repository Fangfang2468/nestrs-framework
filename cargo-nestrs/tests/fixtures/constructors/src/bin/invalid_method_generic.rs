// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct Service;
impl Service {
    #[constructor]
    fn create<T>() -> Self {
        let _ = std::marker::PhantomData::<T>;
        Self
    }
}
fn main() {}
