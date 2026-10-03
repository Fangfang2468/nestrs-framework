#![allow(dead_code)]

use nestrs::injectable;

#[injectable(key = "read")]
struct Database;

#[injectable]
struct OrderService {
    // 裸 inject 请求 default，不会选择 read。
    #[inject]
    database: Database,
}

fn main() {}
