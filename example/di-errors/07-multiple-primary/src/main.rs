#![allow(dead_code)]

use nestrs::{injectable, primary};

trait PaymentGateway: Send + Sync {}

#[primary]
#[injectable]
struct CardGateway;
#[primary]
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
