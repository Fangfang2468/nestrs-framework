use nestrs_core::{ServiceKey, ServiceProvider, ServiceProviderRef};

async fn ordinary_root_methods(provider: &ServiceProvider) {
    let _ = provider.get_required_service::<u32>().await;
    let _ = provider.get_service::<u32>().await;
    let _ = provider
        .get_required_keyed_service::<u32>(ServiceKey::Indexed(1))
        .await;
    let _ = provider
        .get_keyed_service::<u32>(ServiceKey::Indexed(1))
        .await;
}

async fn ordinary_view_methods(provider: ServiceProviderRef<'_>) {
    let _ = provider.get_required_service::<u32>().await;
    let _ = provider.get_service::<u32>().await;
    let _ = provider
        .get_required_keyed_service::<u32>(ServiceKey::Indexed(1))
        .await;
    let _ = provider
        .get_keyed_service::<u32>(ServiceKey::Indexed(1))
        .await;
}

fn main() {}
