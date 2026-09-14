use nestrs_core::ServiceProvider;
use nestrs_macro::injectable;

#[injectable]
struct UserRepository;

impl UserRepository {
    fn display_name(&self) -> &'static str {
        "Ada Lovelace"
    }
}

#[injectable]
struct UserService {
    #[inject]
    repository: UserRepository,
}

impl UserService {
    fn greeting(&self) -> String {
        format!("Hello, {}!", self.repository.display_name())
    }
}

#[injectable]
struct Application {
    #[inject]
    users: UserService,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::<Application>::build()?;

    // Root 是应用的类型化入口。
    println!("{}", provider.root().users.greeting());

    // `get` 只读取这次构建已经激活的默认 concrete Singleton，
    // 不会在这里注册、解析或创建新的服务。
    let users = provider
        .get::<UserService>()
        .expect("Application 的可达闭包已包含 UserService");
    assert_eq!(users.greeting(), "Hello, Ada Lovelace!");

    Ok(())
}
