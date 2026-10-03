#![allow(dead_code)]

use nestrs::injectable;
use nestrs_core::ServiceProvider;

trait Port: Send + Sync {}

#[injectable]
struct First;

#[injectable]
struct Second;

impl Port for First {}
impl Port for Second {}

fn query(provider: &ServiceProvider) {
    drop(provider.get_service::<dyn Port>());
}

fn main() {}
