use nestrs::{bind};

trait ServiceInterface {}

struct Service;

#[bind(primary)]
impl ServiceInterface for Service {}

fn main() {}
