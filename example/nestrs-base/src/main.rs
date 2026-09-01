use nestrs_macro::injectable;

fn cleanup_test() {}

#[injectable(cleanup = "cleanup_test")]
pub struct UserController {

}



fn main() {
    println!("Hello, world!");
}
