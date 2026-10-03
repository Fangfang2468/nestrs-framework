//! 来源不可证明时必须给出 constructor 诊断，不能推测包装或只靠后续类型错误。
#![allow(
    dead_code,
    unused_variables,
    unused_assignments,
    unreachable_code,
    unused_unsafe
)]
use nestrs::{constructor, injectable};
#[injectable]
struct Dependency;
#[injectable]
struct Service {
    db: Dependency,
}
impl Service {
    #[constructor]
    fn new(first: Dependency, second: Dependency) -> Self {
        let (alias,) = (first,);
        Self { db: alias }
    }
}

fn main() {}
