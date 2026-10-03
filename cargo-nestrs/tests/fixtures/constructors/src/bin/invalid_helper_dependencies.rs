//! 普通业务源码即使在同 crate 也不能直接获取工具链内部描述函数。
use nestrs::{constructor, injectable};
#[injectable]
struct Service;
impl Service {
    #[constructor]
    fn create() -> Self {
        Self
    }
}
fn main() {
    let _ = Service::__nestrs_constructor_dependencies;
}
