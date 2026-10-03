#![allow(dead_code)]

use nestrs::factory;

mod model {
    pub struct Database;
}

use model::Database as FirstName;
use model::Database as SecondName;

#[factory]
fn first() -> FirstName {
    FirstName
}

#[factory]
fn second() -> SecondName {
    SecondName
}

fn main() {}
