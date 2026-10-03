#![allow(dead_code)]

use nestrs::injectable;

#[injectable(lifetime = Scoped)]
struct Session;

#[injectable(lifetime = Transient)]
struct Intermediate {
    // 故意错误：lazy 边仍传播 scope 要求，经 Transient 传给上游 Singleton。
    #[inject]
    #[lazy]
    session: Session,
}

#[injectable]
struct Application {
    #[inject]
    intermediate: Intermediate,
}

fn main() {}
