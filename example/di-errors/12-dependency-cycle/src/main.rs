#![allow(dead_code)]

use nestrs::injectable;

#[injectable]
struct OrderService {
    #[inject]
    payment: PaymentService,
}

#[injectable]
struct PaymentService {
    #[inject]
    order: OrderService,
}

fn main() {}
