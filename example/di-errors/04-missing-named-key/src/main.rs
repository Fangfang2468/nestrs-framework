#![allow(dead_code)]

use nestrs::injectable;

trait PaymentGateway: Send + Sync {}

#[injectable(key = "sandbox")]
struct SandboxGateway;
impl PaymentGateway for SandboxGateway {}

#[injectable]
struct Checkout {
    #[inject("live")]
    gateway: dyn PaymentGateway,
}

fn main() {}
