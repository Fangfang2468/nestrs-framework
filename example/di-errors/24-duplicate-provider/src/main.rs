#![allow(dead_code)]

use nestrs::{factory, primary};

struct Database;

#[factory]
fn database() -> Database {
    Database
}

// primary 不允许覆盖同一 concrete 类型与 key 的另一个 provider。
#[primary]
#[factory]
fn fallback_database() -> Database {
    Database
}

fn main() {}
