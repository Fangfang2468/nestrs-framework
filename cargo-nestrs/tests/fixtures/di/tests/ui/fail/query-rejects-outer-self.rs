use nestrs_core::ServiceProvider;

struct Value;

impl Value {
    async fn query(provider: &ServiceProvider) {
        let _ = nestrs_core::get_service!(provider, Self).await;
    }
}

fn main() {}
