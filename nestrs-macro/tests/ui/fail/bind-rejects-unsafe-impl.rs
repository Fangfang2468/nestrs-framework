use nestrs_macro::bind;

unsafe trait Marker {}
struct Service;

#[bind]
unsafe impl Marker for Service {}

fn main() {}
