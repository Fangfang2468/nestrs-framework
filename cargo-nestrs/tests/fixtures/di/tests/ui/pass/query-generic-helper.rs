use nestrs_core::ServiceProvider;

async fn generic_query<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    let _ = provider.get_service::<T>().await;
}

fn main() {}
