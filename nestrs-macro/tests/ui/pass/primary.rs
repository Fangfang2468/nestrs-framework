use nestrs_macro::primary;

trait TraitTest1 {}
trait TraitTest2 {}

#[primary]
struct DefaultPrimary;

#[primary()]
struct ExplicitDefaultPrimary;

#[primary(TraitTest1, TraitTest2)]
struct TraitPrimary;

fn main() {}
