#![allow(dead_code)]

use nestrs::{injectable, primary};

trait PaymentGateway: Send + Sync {}

#[injectable]
struct CardGateway;
#[injectable]
struct BankGateway;
impl PaymentGateway for CardGateway {}
impl PaymentGateway for BankGateway {}

#[primary]
#[injectable(key = "preferred")]
struct PreferredGateway;
impl PaymentGateway for PreferredGateway {}

#[injectable]
struct Checkout {
    #[inject]
    gateway: dyn PaymentGateway,
}

fn main() {}
