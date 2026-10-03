use nestrs_core::ServiceProvider;

async fn close_while_borrowed(provider: ServiceProvider) {
    let service = provider.get_required_service::<String>()
        .await
        .unwrap();
    provider.dispose_async().await.unwrap();
    println!("{service}");
}

fn main() {}
