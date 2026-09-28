use nestrs::{primary};

trait TraitTest {}

#[primary(TraitTest)]
struct Service;

fn main() {}
