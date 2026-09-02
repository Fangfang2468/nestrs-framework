use nestrs_macro::bind;

trait ServiceInterface {}

struct Service;

#[bind(primary)]
impl ServiceInterface for Service {}

fn main() {}
