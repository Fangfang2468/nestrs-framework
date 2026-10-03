#![allow(dead_code)]

use nestrs::injectable;

struct Missing;

#[injectable]
struct Database;

macro_rules! declare_service {
    () => {
        #[injectable]
        struct Application {
            #[cfg(any())]
            #[inject]
            missing: Missing,
            #[inject]
            database: Database,
        }
    };
}

declare_service!();

fn main() {}
