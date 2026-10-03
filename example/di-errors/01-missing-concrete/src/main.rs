#![allow(dead_code)]

use nestrs::injectable;

struct Database;

#[injectable]
struct OrderService {
    // 故意缺少 Database 的 provider。
    #[inject]
    database: Database,
}

fn main() {}
