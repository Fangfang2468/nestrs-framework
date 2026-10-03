use nestrs::injectable;
#[injectable]
struct First {
    #[inject]
    #[lazy]
    second: Second,
}
#[injectable]
struct Second {
    #[inject]
    first: First,
}
fn main() {}
