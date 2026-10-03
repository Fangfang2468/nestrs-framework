use nestrs::injectable;
trait Port: Send + Sync {}
#[injectable]
struct First;
#[injectable]
struct Second;
impl Port for First {}
impl Port for Second {}
#[injectable]
struct Consumer {
    #[inject]
    optional: Option<dyn Port>,
}
fn main() {}
