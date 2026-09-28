use nestrs_core::ServiceScope;

async fn close_while_borrowed(scope: ServiceScope<'_>) {
    let service = nestrs_core::get_required_service!(scope.service_provider(), String)
        .await
        .unwrap();
    scope.dispose_async().await.unwrap();
    println!("{service}");
}

fn main() {}
