#![allow(dead_code)]

use nestrs::injectable;

trait PaymentGateway: Send + Sync {}

#[injectable]
struct Checkout {
    #[inject]
    gateway: dyn PaymentGateway,
}

fn main() {}
