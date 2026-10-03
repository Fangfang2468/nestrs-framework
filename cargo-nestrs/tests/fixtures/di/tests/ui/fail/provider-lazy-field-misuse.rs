use nestrs::{factory, injectable};
#[injectable]
struct Arguments {
    #[inject]
    #[lazy(false)]
    item: u32,
}
#[injectable]
struct DefaultField {
    #[lazy]
    item: u32,
}
#[factory]
fn parameter(#[lazy(true)] item: u32) -> u64 {
    u64::from(*item)
}
fn main() {}
