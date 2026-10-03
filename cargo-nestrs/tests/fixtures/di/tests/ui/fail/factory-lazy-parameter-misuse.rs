use nestrs::factory;

#[factory]
fn boolean(#[lazy(false)] item: u32) -> u64 {
    0
}
#[factory]
fn empty(#[lazy()] item: u32) -> u64 {
    0
}
#[factory]
fn duplicate(
    #[lazy]
    #[lazy]
    item: u32,
) -> u64 {
    0
}
fn main() {}
