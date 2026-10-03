// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct Service {
    #[value(7)]
    id: usize,
}
impl Service {
    #[constructor]
    fn create() -> Self {
        Self { id: 7 }
    }
}
fn main() {}
