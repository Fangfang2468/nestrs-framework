use nestrs_macro::injectable;


#[injectable]
pub struct UserService {
    db: String,
}

#[injectable(lifetime = Scoped)]
pub struct UserController {
    db: String,
}

#[injectable(lifetime = Lifetime::Transient, key = "1")]
pub struct UserRepository {
    db: String,
}

fn main() {}