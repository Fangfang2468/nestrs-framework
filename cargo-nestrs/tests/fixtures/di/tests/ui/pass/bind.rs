use nestrs::{bind, injectable};

trait ServiceInterface: Send + Sync {}

#[injectable]
struct Service;

#[bind]
impl ServiceInterface for Service {}

fn main() {}
