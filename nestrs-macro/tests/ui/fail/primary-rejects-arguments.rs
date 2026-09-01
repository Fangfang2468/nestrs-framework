use nestrs_macro::primary;

trait TraitTest {}

#[primary(TraitTest)]
struct Service;

fn main() {}
