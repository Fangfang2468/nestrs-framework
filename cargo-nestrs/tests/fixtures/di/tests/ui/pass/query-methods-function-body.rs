use nestrs::injectable;
use nestrs_core::{ServiceKey, ServiceProvider, ServiceScope};

use std::marker::PhantomData;

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
}
struct User;
type UserRepository = Repository<User>;
struct Missing;

async fn query_from_function_body(provider: &ServiceProvider, scope: &ServiceScope<'_>) {
    let _ = provider.get_required_service::<UserRepository>().await;
    let _ = scope
        .service_provider()
        .get_required_service::<Repository<User>>()
        .await;
    let _ = provider.get_service::<Missing>().await;
    let dynamic_key = ServiceKey::Named(String::from("tenant"));
    let _ = provider.get_keyed_service::<Missing>(dynamic_key).await;
}

impl User {
    async fn query_from_method(provider: &ServiceProvider) {
        let _ = provider.get_required_service::<UserRepository>().await;
    }
}

fn main() {}
