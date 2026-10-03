//! 函数项别名不能绕过工具链内部 adapter 的来源审计。
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
    let _ = Service::__nestrs_constructor_activate;
}
