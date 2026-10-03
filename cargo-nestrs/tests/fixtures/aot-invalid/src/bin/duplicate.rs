use nestrs::factory;
struct Service;
#[factory]
fn first() -> Service {
    panic!("must not execute")
}
#[factory]
fn second() -> Service {
    panic!("must not execute")
}
fn main() {}
