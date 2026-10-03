#![allow(dead_code, non_snake_case)]

use nestrs::{constructor, injectable};

struct Database;

#[injectable]
struct Application {}

impl Application {
    #[constructor]
    fn Application(_database: Database) -> Self {
        Application {}
    }
}

fn main() {}
