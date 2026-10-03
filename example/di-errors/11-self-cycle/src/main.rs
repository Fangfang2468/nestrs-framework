#![allow(dead_code)]

use nestrs::injectable;

#[injectable]
struct Cache {
    #[inject]
    parent: Cache,
}

fn main() {}
