use nestrs::{injectable};
use nestrs_core::{
    ServiceKey, ServiceProvider, ServiceScope, get_required_service as required,
    get_service as optional,
};

use std::marker::PhantomData;

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
}
struct User;
type UserRepository = Repository<User>;
struct Missing;

async fn query_from_function_body(provider: &ServiceProvider, scope: &ServiceScope<'_>) {
    let _ = required!(provider, UserRepository).await;
    let _ = required!(scope.service_provider(), Repository<User>).await;
    let _ = optional!(provider, Missing).await;
    let dynamic_key = ServiceKey::Named(String::from("tenant"));
    let _ = nestrs_core::get_keyed_service!(provider, Missing, dynamic_key).await;
}

impl User {
    async fn query_from_method(provider: &ServiceProvider) {
        let _ = required!(provider, UserRepository).await;
    }
}

fn main() {}
