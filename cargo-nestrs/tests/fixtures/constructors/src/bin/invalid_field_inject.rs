// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct Dependency;
#[injectable]
struct Service {
    #[inject]
    dependency: Dependency,
}
impl Service {
    #[constructor]
    fn create(dependency: Dependency) -> Self {
        Self { dependency }
    }
}
fn main() {}
