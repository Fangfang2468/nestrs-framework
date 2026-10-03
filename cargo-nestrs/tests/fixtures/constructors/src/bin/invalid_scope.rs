// 此入口必须在 cargo nestrs check 阶段拒绝，不运行构造器。
use nestrs::{constructor, injectable};
#[injectable(lifetime = Scoped)]
struct Session;
#[injectable(lifetime = Singleton)]
struct Service;
impl Service {
    #[constructor]
    fn create(_session: Session) -> Self {
        Self
    }
}
fn main() {}
