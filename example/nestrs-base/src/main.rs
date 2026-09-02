use nestrs_core::registration::service_collection::ServiceCollection;
use nestrs_macro::{bind, factory, injectable, primary};


trait GetUser: Send + Sync {
    fn get_user(&self) -> String;
}


async fn cleanup_test() {}

#[injectable(cleanup = "cleanup_test")]
#[primary]
pub struct UserController {

}

#[bind]
impl GetUser for UserController {

    fn get_user(&self) -> String {
        "John Doe".into()
    }
}



#[factory]
pub fn use_factory() {
}


fn main() {
    let collection = ServiceCollection::new();
}
