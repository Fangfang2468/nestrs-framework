use nestrs::{injectable};


#[injectable]
pub struct UserService {
    db: String,
}

#[injectable(lifetime = Scoped)]
pub struct UserController {
    db: String,
}

#[injectable(lifetime = ServiceLifetime::Transient, key = "1")]
pub struct UserRepository {
    db: String,
}

fn main() {}
