#![allow(dead_code)]

use nestrs::{bind, injectable};

trait Port: Send + Sync {}

#[injectable]
struct Alpha;

#[injectable]
struct Beta;

#[bind]
impl Port for Alpha {}

#[bind]
impl Port for Beta {}

fn main() {}
