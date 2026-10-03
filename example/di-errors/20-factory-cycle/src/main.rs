#![allow(dead_code)]

use nestrs::factory;

struct First;
struct Second;

// 故意形成 factory 参数环；即使函数体不使用参数，依赖关系仍然存在。
#[factory]
fn first(_second: Second) -> First {
    First
}

#[factory]
fn second(_first: First) -> Second {
    Second
}

fn main() {}
