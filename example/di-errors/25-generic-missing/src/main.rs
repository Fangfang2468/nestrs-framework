#![allow(dead_code)]

use nestrs::injectable;
use std::marker::PhantomData;

struct User;
struct Database;

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
    // 故意遗漏 Database 的 provider。
    #[inject]
    database: Database,
}

// 此注入显式触达 Repository<User>，使泛型蓝图闭合并检查内部依赖。
#[injectable]
struct Application {
    #[inject]
    repository: Repository<User>,
}

fn main() {}
