use nestrs::{injectable};

fn cleanup() {}

#[injectable(cleanup = "cleanup")]
pub struct Service;

fn main() {}
