// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct Service;
impl Service {
    #[constructor(anything)]
    fn create() -> Self {
        Self
    }
}
fn main() {}
