#![forbid(unsafe_code)]

use std::marker::PhantomData;

use nestrs_core::__private::{ComponentDefinition, Inject};
use nestrs_macro::injectable;

struct User;

#[injectable]
struct Repository<Entity> {
    marker: PhantomData<Entity>,
}

#[injectable]
struct UserService {
    #[inject]
    repository: Repository<User>,
}

fn accepts_injected_repository(_: Inject<Repository<User>>) {}

fn verifies_macro_contract(service: UserService) {
    accepts_injected_repository(service.repository);
    let _ = <Repository<User> as ComponentDefinition>::component();
}

fn main() {}
