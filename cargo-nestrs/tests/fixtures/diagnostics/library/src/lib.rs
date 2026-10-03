#![allow(dead_code)]

use nestrs::injectable;
use std::marker::PhantomData;

struct Database;

#[injectable]
pub struct Repository<T> {
    marker: PhantomData<T>,
    #[inject]
    database: Database,
}
