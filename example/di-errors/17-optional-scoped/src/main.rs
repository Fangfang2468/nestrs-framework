#![allow(dead_code)]

use nestrs::injectable;

trait SessionPort: Send + Sync {}

#[injectable(lifetime = Scoped)]
struct Session;

impl SessionPort for Session {}

#[injectable]
struct Application {
    // 故意错误：trait 唯一候选 Session 是 Scoped；optional 和投影均不改变其生命周期。
    #[inject]
    session: Option<dyn SessionPort>,
}

fn main() {}
