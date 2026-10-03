use nestrs_core::ServiceProvider;

struct Value;

impl Value {
    async fn query(provider: &ServiceProvider) {
        let _ = provider.get_service::<Self>().await;
    }
}

fn main() {}
