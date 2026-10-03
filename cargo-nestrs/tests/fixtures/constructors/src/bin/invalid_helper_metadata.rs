//! 普通业务源码不能依赖 constructor 的内部字段映射元数据。
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
    let _ = Service::__NESTRS_CONSTRUCTOR;
}
