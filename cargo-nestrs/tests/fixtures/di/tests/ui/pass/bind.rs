use nestrs::{bind};

trait ServiceInterface: Send + Sync {}

struct Service;

#[bind]
impl ServiceInterface for Service {}

fn main() {}
