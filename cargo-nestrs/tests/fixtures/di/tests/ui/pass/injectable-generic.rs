#![forbid(unsafe_code)]
use nestrs::{injectable};
use std::marker::PhantomData;

use nestrs_core::__private::{Injection, ProviderDefinition};


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

fn accepts_injected_repository(_: Injection<Repository<User>>) {}

fn verifies_macro_contract(service: UserService) {
    accepts_injected_repository(service.repository);
    let _ = <Repository<User> as ProviderDefinition>::provider();
}

fn main() {}
