#![forbid(unsafe_code)]
use nestrs::injectable;
use std::marker::PhantomData;

use nestrs_core::{Injection, ServiceProvider};

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
}

async fn resolves_closed_repository(provider: &ServiceProvider) {
    let _: &Repository<User> = provider
        .get_required_service::<Repository<User>>()
        .await
        .unwrap();
}

fn main() {}
