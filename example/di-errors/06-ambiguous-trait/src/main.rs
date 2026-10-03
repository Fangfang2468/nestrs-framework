#![allow(dead_code)]

use nestrs::injectable;

trait PaymentGateway: Send + Sync {}

#[injectable]
struct CardGateway;
#[injectable]
struct BankGateway;
impl PaymentGateway for CardGateway {}
impl PaymentGateway for BankGateway {}

#[injectable]
struct Checkout {
    #[inject]
    gateway: dyn PaymentGateway,
}

fn main() {}
