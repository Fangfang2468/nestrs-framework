use nestrs::{factory, injectable};
use nestrs_core::LazyInjection;

#[injectable]
struct Dependency;
struct Missing;
struct Service {
    dependency: LazyInjection<Dependency>,
    optional: Option<LazyInjection<Missing>>,
}

// 只有 lazy 参数的函数不需要 factory frame 生命周期；token 按值交付。
#[factory]
fn create(#[lazy] dependency: Dependency, #[nestrs::lazy] optional: Option<Missing>) -> Service {
    Service {
        dependency,
        optional,
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build(None).await.unwrap();
    let service = provider.get_required_service::<Service>().await.unwrap();
    assert!(service.optional.is_none());
    service.dependency.get().await.unwrap();
    provider.dispose_async().await.unwrap();
}
