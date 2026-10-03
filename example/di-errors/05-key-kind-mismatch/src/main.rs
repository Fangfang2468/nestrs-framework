#![allow(dead_code)]

use nestrs::injectable;

#[injectable(key = "7")]
struct Queue;

#[injectable]
struct Worker {
    #[inject(7)]
    queue: Queue,
}

fn main() {}
