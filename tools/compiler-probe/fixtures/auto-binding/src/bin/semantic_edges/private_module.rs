use nestrs::injectable;
use nestrs_core::ServiceProvider;

#[injectable]
struct PrivateService {
    #[value(13)]
    value: usize,
}

mod api {
    use super::{PrivateService, ServiceProvider};

    trait PrivatePort: Send + Sync {
        fn value(&self) -> usize;
        fn identity(&self) -> usize;
    }

    impl PrivatePort for PrivateService {
        fn value(&self) -> usize {
            self.value
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    pub(super) async fn assert_projection(provider: &ServiceProvider) {
        let concrete = nestrs_core::get_required_service!(provider, PrivateService)
            .await
            .unwrap();
        let projected = nestrs_core::get_required_service!(provider, dyn PrivatePort)
            .await
            .unwrap();
        assert_eq!(
            projected.identity(),
            concrete as *const PrivateService as usize
        );
        assert_eq!(projected.value(), 13);
    }
}

pub(crate) async fn assert_private_projection(provider: &ServiceProvider) {
    api::assert_projection(provider).await;
}
