// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct Service;
impl Service {
    #[constructor]
    fn first() -> Self {
        Self
    }
    #[constructor]
    fn second() -> Self {
        Self
    }
}
fn main() {}
