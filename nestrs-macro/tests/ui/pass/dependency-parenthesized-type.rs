use nestrs_macro::{factory, injectable};

struct Service;
struct Made;

// 括号包裹不是元组：`(Service)` 与 `Service` 在两个宏入口下都必须等价。
#[injectable]
struct Consumer {
    #[inject]
    service: (Service),
}

#[factory]
fn make(#[inject] service: (Service)) -> Made {
    let _ = &*service;
    Made
}

fn main() {}
