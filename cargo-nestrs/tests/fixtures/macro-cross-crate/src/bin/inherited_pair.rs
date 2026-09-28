use nestrs_core::{ServiceProvider, get_required_service};
use nestrs_macro_cross_crate::{KnownPort, KnownUser, Repository};

type PublicAlias = Repository<KnownUser>;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let concrete = get_required_service!(provider, PublicAlias).await.unwrap();
    let port = get_required_service!(provider, dyn KnownPort)
        .await
        .unwrap();
    assert_eq!(port.value(), 23);
    assert!(std::ptr::addr_eq(
        concrete as *const PublicAlias,
        port as *const dyn KnownPort,
    ));
    provider.dispose_async().await.unwrap();
    println!("inherited binding: upstream projection reused without duplicate registration");
}
