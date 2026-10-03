#![allow(dead_code)]

use nestrs::factory;

struct RequestSession;
struct Application;

#[factory(lifetime = Scoped)]
fn session() -> RequestSession {
    RequestSession
}

// 故意让 Singleton factory 依赖 Scoped 服务，编译阶段即拒绝。
#[factory(lifetime = Singleton)]
fn application(_session: RequestSession) -> Application {
    Application
}

fn main() {}
