#![allow(dead_code)]

use nestrs::factory;

struct Database;
struct Application;

// 故意遗漏 Database 的 provider：factory 的必选参数同样参与图验证。
#[factory]
fn application(_database: Database) -> Application {
    Application
}

fn main() {}
