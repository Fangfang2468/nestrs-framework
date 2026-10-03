#![allow(dead_code)]

use diagnostics_library::Repository;
use nestrs::injectable;

struct User;

#[injectable]
struct Application {
    #[inject]
    repository: Repository<User>,
}

fn main() {}
