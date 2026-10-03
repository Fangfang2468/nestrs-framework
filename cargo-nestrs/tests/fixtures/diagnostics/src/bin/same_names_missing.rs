#![allow(dead_code)]

use nestrs::injectable;

mod available {
    #[nestrs::injectable]
    pub struct Database;
}

mod missing {
    pub struct Database;
}

use available::Database as AvailableDatabase;
use missing::Database as RequestedDatabase;

#[injectable]
struct Application {
    #[inject]
    ready: AvailableDatabase,
    #[inject]
    missing: RequestedDatabase,
}

fn main() {}
