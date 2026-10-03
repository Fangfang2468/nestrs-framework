#![allow(dead_code)]

use nestrs::injectable;

struct Database;
struct Queue;

#[injectable]
struct First {
    #[inject]
    dependency: Database,
}

#[injectable]
struct Second {
    #[inject]
    dependency: Queue,
}

fn main() {}
