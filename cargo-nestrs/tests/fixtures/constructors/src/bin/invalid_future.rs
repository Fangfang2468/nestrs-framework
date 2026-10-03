// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct Service;
impl Service {
    #[constructor]
    fn create() -> impl std::future::Future<Output = Self> {
        async { Self }
    }
}
fn main() {}
