use nestrs_macro::injectable;

trait Interface {}

#[injectable]
#[nestrs_macro::primary(Interface)]
struct Service;

fn main() {}
