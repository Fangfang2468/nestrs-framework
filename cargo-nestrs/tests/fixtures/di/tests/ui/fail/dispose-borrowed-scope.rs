use nestrs_core::ServiceScope;

async fn close_while_borrowed(scope: ServiceScope<'_>) {
    let service = scope.service_provider().get_required_service::<String>()
        .await
        .unwrap();
    scope.dispose_async().await.unwrap();
    println!("{service}");
}

fn main() {}
