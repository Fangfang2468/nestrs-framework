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
        let concrete = provider
            .get_required_service::<PrivateService>()
            .await
            .unwrap();
        let projected = provider
            .get_required_service::<dyn PrivatePort>()
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
