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
pub fn use_factory(
    // #[inject] user_controller: UserController,   // 👈 普通注入 方式一
    // user_controller2: UserController,           // 👈 普通注入 方式二
    // #[inject("key")] named_key_user_controller: UserController, // 👈 带 key 的注入（具名服务注入）
    // #[inject(12)] indexed_key_user_controller: UserController, // 👈 带 key 的注入（key类型为数字）
    // optional_user_controller: Option<UserController>, // 👈 可选注入

    // interface_user_controller: dyn GetUser, // 👈 接口注入（注入方式上的多态）
    // #[inject("interface")] interface_user_controller2: dyn GetUser, // 👈 接口注入（注入方式上的多态，带 key 的注入）
) -> Result<(), std::io::Error> {
    Ok(())
}


fn main() {
    let collection = ServiceCollection::new();
}
