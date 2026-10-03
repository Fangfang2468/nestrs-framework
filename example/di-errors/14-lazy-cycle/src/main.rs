#![allow(dead_code)]

use nestrs::injectable;

#[injectable]
struct Alpha {
    // 故意错误：延迟边仍属于完整依赖图，不能用 lazy 打断 Alpha -> Beta -> Alpha。
    #[inject]
    #[lazy]
    beta: Beta,
}

#[injectable]
struct Beta {
    #[inject]
    alpha: Alpha,
}

fn main() {}
