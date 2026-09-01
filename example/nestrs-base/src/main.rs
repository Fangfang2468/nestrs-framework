use nestrs_macro::{constructor, factory, injectable, primary};

struct A;

trait TraitTest1 {
}

trait TraitTest2 {
}


async fn cleanup_test() {}

#[injectable(cleanup = "cleanup_test")]
#[primary]
pub struct UserController {

}

impl UserController {

    #[constructor]
    pub fn new() -> Self {
        UserController {}
    }
}

#[factory]
pub fn use_factory() {
}


fn main() {
    println!("Hello, world!");
}
