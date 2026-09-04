use nestrs_core::registration::service_collection::ServiceCollection;
use nestrs_macro::{bind, factory, injectable, primary};

const BASE_RETRIES: u64 = 2;
static STATIC_RETRY_INCREMENT: u64 = 1;

fn prefixed_name(prefix: &str) -> String {
    format!("{prefix}-controller")
}

#[derive(Debug)]
#[injectable]
pub struct Values {
    #[value("123".to_owned())]
    pub name: String,

    #[value("full_name")]
    pub full_name: String,

    #[value(3)]
    pub timestamp: u64,

    #[value(BASE_RETRIES + STATIC_RETRY_INCREMENT)]
    pub retries: u64,

    #[value(prefixed_name("user"))]
    pub label: String,

    #[value({
        let seed = 40;
        seed + 2
    })]
    pub answer: usize,

    pub default_value: Vec<String>,
}

#[injectable]
pub struct TupleValues(#[value(1 + 2)] pub usize, #[value("ok")] pub &'static str);

pub trait IUserCollection: Send + Sync {
    fn get_users(&self) -> Vec<UserEntity>;
}

#[derive(Debug, Clone)]
pub struct UserEntity {
    pub id: u32,

    pub name: String,

    pub age: u32,
}

#[injectable]
pub struct Repository<Entity> {
    _marker: std::marker::PhantomData<Entity>,
}

impl<Entity> Repository<Entity> {
    pub fn find_all(&self) -> Vec<UserEntity> {
        vec![
            UserEntity {
                id: 1,
                name: "John Doe".into(),
                age: 30,
            },
            UserEntity {
                id: 2,
                name: "Jane Smith".into(),
                age: 25,
            },
        ]
    }
}

#[injectable]
pub struct UserService {
    #[inject]
    pub repository: Repository<UserEntity>,
}

#[bind]
impl IUserCollection for UserService {
    fn get_users(&self) -> Vec<UserEntity> {
        self.repository.find_all()
    }
}

impl UserService {
    pub fn total_users(&self) -> usize {
        self.repository.find_all().len()
    }
}

async fn cleanup_test() {}

#[injectable(cleanup = "cleanup_test")]
#[primary]
pub struct UserController {
    #[inject]
    pub user_service: UserService,

    #[inject]
    pub user_collection_getter: dyn IUserCollection,

    #[value("123")]
    pub name: String,

    #[value(3)]
    pub timestamp: u64,
}

impl UserController {
    pub fn get_all_users(&self) -> Vec<UserEntity> {
        let users = self.user_collection_getter.get_users();
        println!(
            "UserController: Retrieved {} users",
            self.user_service.total_users()
        );
        println!("{users:#?}");
        users
    }
}

// #[inject] user_controller: UserController,   // 👈 普通注入 方式一
// user_controller2: UserController,            // 👈 普通注入 方式二
// #[inject("key")] named_key_user_controller: UserController, // 👈 带 key 的注入（具名服务注入）
// #[inject(12)] indexed_key_user_controller: UserController, // 👈 带 key 的注入（key类型为数字）
// optional_user_controller: Option<UserController>, // 👈 可选注入
// interface_user_controller: dyn IUserCollection, // 👈 接口注入（注入方式上的多态）
// #[inject("interface")] interface_user_controller2: dyn IUserCollection, // 👈 接口注入（具名服务注入）
#[factory]
pub fn use_factory() -> Result<u32, std::io::Error> {
    Ok(1)
}

fn main() {
    let collection = ServiceCollection::new();
    let arena = collection
        .instantiate::<UserController>()
        .expect("UserController、UserService 及其 Repository<UserEntity> 依赖应可直接实例化");
    let controller = arena
        .get::<UserController>()
        .expect("UserController 应已提交到本次实例化 Arena");

    let users = controller.get_all_users();
    assert_eq!(users.len(), 2);
}
