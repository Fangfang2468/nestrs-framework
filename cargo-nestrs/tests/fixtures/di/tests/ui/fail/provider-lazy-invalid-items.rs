use nestrs::lazy;
#[lazy]
struct NoInjectable;
#[lazy(false)]
fn no_factory() {}
#[lazy]
enum WrongEnum {
    Variant,
}
#[lazy]
const WRONG_CONST: usize = 1;
struct Plain;
#[lazy]
impl Plain {}
fn main() {}
