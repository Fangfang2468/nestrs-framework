use nestrs::{injectable, primary};
trait Port: Send + Sync {}
#[primary]
#[injectable]
struct First;
#[primary]
#[injectable]
struct Second;
impl Port for First {}
impl Port for Second {}
#[injectable]
struct Consumer {
    #[inject]
    port: dyn Port,
}
fn main() {}
