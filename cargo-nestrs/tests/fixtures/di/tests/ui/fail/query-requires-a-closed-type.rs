use nestrs_core::ServiceProvider;

async fn generic_query<T: Send + Sync + 'static>(provider: &ServiceProvider) {
    let _ = nestrs_core::get_service!(provider, T).await;
}

fn main() {}
