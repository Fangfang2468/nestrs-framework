use nestrs::{injectable};

async fn cleanup() {}

#[injectable(cleanup = "cleanup")]
pub struct Service;

fn main() {}
