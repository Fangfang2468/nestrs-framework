#![allow(dead_code)]

use nestrs::{constructor, injectable};

struct Database;

#[injectable]
struct Application;

impl Application {
    // 故意遗漏 Database 的 provider；constructor 参数是此服务的依赖来源。
    #[constructor]
    fn new(_database: Database) -> Self {
        Self
    }
}

fn main() {}
