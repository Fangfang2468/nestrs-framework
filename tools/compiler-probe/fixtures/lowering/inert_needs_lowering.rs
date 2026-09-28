#![allow(dead_code)]

trait Port: Send + Sync {}

// Legal tool attributes do not turn this into Injection<dyn Port>.
#[nestrs::injectable]
struct Consumer {
    port: dyn Port,
    value: usize,
}

// The same applies to converting factory parameters into frame borrows.
#[nestrs::factory]
async fn make(port: dyn Port) -> usize {
    let _ = port;
    1
}

fn main() {}
