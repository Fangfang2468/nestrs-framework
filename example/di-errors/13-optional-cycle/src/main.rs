#![allow(dead_code)]

use nestrs::injectable;

#[injectable]
struct Alpha {
    // 故意错误：Beta 已注册，optional 边真实存在，不会自动交付 None 来打断环。
    #[inject]
    beta: Option<Beta>,
}

#[injectable]
struct Beta {
    #[inject]
    alpha: Alpha,
}

fn main() {}
