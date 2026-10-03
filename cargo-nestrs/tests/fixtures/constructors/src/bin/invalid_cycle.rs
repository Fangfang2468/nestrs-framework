// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable]
struct First;
#[injectable]
struct Second;
impl First {
    #[constructor]
    fn create(_second: Second) -> Self {
        Self
    }
}
impl Second {
    #[constructor]
    fn create(_first: First) -> Self {
        Self
    }
}
fn main() {}
