// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
struct Missing;
#[injectable]
struct Service;
impl Service {
    #[constructor]
    fn create(_missing: Missing) -> Self {
        Self
    }
}
fn main() {}
