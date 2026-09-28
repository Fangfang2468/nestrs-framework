use nestrs::{injectable, primary};

trait Interface {}

#[injectable]
#[primary(Interface)]
struct Service;

fn main() {}
