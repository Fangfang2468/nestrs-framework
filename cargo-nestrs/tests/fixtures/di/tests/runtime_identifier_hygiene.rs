//! Generated bindings must stay distinct from legal business constants and raw identifiers.
#![allow(non_upper_case_globals, non_camel_case_types)]
use nestrs_core::ServiceProvider;

mod constructor_error {
    use nestrs::{constructor, injectable};
    pub const error: usize = 47;
    #[injectable]
    pub struct Success {
        pub value: usize,
    }
    impl Success {
        #[constructor]
        fn new() -> Result<Self, &'static str> {
            Ok(Self { value: error })
        }
    }
    #[injectable]
    pub struct Failure;
    impl Failure {
        #[constructor]
        fn new() -> Result<Self, &'static str> {
            Err("hygienic constructor failure")
        }
    }
}

mod constructor_inputs {
    use nestrs::{constructor, injectable};
    pub const __nestrs_inputs: usize = 53;
    #[injectable]
    struct Dependency;
    #[injectable]
    pub struct Service {
        _dependency: Dependency,
        pub value: usize,
    }
    impl Service {
        #[constructor]
        fn new(dependency: Dependency) -> Self {
            Self {
                _dependency: dependency,
                value: __nestrs_inputs,
            }
        }
    }
}

mod constructor_instance {
    use nestrs::{constructor, injectable};
    pub const __nestrs_instance: usize = 59;
    #[injectable]
    pub struct Service {
        pub value: usize,
    }
    impl Service {
        #[constructor]
        fn new() -> Self {
            Self {
                value: __nestrs_instance,
            }
        }
    }
}

mod automatic_context {
    use nestrs::injectable;
    pub const __nestrs_injectable_context_for_Service: usize = 61;
    #[injectable]
    pub struct Service {
        #[value(__nestrs_injectable_context_for_Service)]
        pub value: usize,
    }
}

mod automatic_instance {
    use nestrs::injectable;
    pub const __nestrs_injectable_instance: usize = 67;
    #[injectable]
    pub struct Service {
        #[value(__nestrs_injectable_instance)]
        pub value: usize,
    }
}

mod generic_automatic {
    use nestrs::injectable;
    pub const __nestrs_injectable_context_for_Service: usize = 71;
    pub const __nestrs_injectable_instance: usize = 73;
    #[injectable]
    pub struct Dependency;
    #[injectable]
    pub struct Service<T: Send + Sync + 'static> {
        #[inject]
        pub dependency: T,
        #[value(__nestrs_injectable_context_for_Service + __nestrs_injectable_instance)]
        pub value: usize,
    }
}

mod configured_automatic {
    use nestrs::injectable;
    pub const __nestrs_injectable_context_for_Service: usize = 97;
    pub const __nestrs_injectable_instance: usize = 101;
    #[injectable]
    pub struct Service {
        #[cfg(any())]
        removed: MissingType,
        #[cfg(all())]
        #[value(__nestrs_injectable_context_for_Service + __nestrs_injectable_instance)]
        pub value: usize,
    }
}

mod generic_constructor {
    use nestrs::{constructor, injectable};
    const __nestrs_inputs: usize = 103;
    const __nestrs_instance: usize = 107;
    const error: usize = 109;
    #[injectable]
    pub struct Service<T: Send + Sync + 'static> {
        pub dependency: T,
        pub value: usize,
    }
    impl<T: Send + Sync + 'static> Service<T> {
        #[constructor]
        fn new(dependency: T) -> Result<Self, &'static str> {
            Ok(Self {
                dependency,
                value: __nestrs_inputs + __nestrs_instance + error,
            })
        }
    }
}

mod static_bindings {
    use nestrs::{constructor, injectable};
    static __nestrs_inputs: usize = 113;
    static __nestrs_instance: usize = 127;
    static error: usize = 131;
    static __nestrs_injectable_context_for_Automatic: usize = 137;
    static __nestrs_injectable_instance: usize = 139;

    #[injectable]
    pub struct Automatic {
        #[value(__nestrs_injectable_context_for_Automatic + __nestrs_injectable_instance)]
        pub value: usize,
    }
    #[injectable]
    pub struct Explicit {
        pub value: usize,
    }
    impl Explicit {
        #[constructor]
        fn new() -> Result<Self, &'static str> {
            Ok(Self {
                value: __nestrs_inputs + __nestrs_instance + error,
            })
        }
    }
}

mod raw_injectable {
    use nestrs::injectable;
    #[injectable]
    pub struct r#type;
    #[injectable]
    pub struct r#struct<T: Send + Sync + 'static> {
        #[inject]
        pub dependency: T,
        #[value(79)]
        pub r#type: usize,
    }
}

mod raw_factory {
    use nestrs::factory;
    pub struct Sync(pub usize);
    pub struct Async(pub usize);
    pub struct Future(pub usize);
    pub struct Failure;
    #[factory]
    fn r#type() -> Sync {
        Sync(83)
    }
    #[factory]
    async fn r#async(value: Sync) -> Result<Async, &'static str> {
        tokio::task::yield_now().await;
        Ok(Async(value.0 + 2))
    }
    #[factory]
    fn r#await(value: Sync) -> impl std::future::Future<Output = Future> {
        async move {
            tokio::task::yield_now().await;
            Future(value.0 + 4)
        }
    }
    #[factory]
    fn r#try() -> Result<Failure, &'static str> {
        Err("raw factory failure")
    }
}

mod raw_constructor {
    use nestrs::{constructor, injectable};
    #[injectable]
    pub struct Service {
        pub value: usize,
    }
    impl Service {
        #[constructor]
        fn r#type() -> Result<Self, &'static str> {
            Ok(Self { value: 89 })
        }
    }
}

#[tokio::test]
async fn class_bindings_preserve_business_constants_in_all_constructor_modes() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(
        provider
            .get_required_service::<constructor_error::Success>()
            .await
            .unwrap()
            .value,
        47
    );
    let failure = match provider
        .get_required_service::<constructor_error::Failure>()
        .await
    {
        Ok(_) => panic!("fallible constructor must retain its error"),
        Err(error) => error,
    };
    assert!(failure.to_string().contains("hygienic constructor failure"));
    assert_eq!(
        provider
            .get_required_service::<constructor_inputs::Service>()
            .await
            .unwrap()
            .value,
        53
    );
    assert_eq!(
        provider
            .get_required_service::<constructor_instance::Service>()
            .await
            .unwrap()
            .value,
        59
    );
    assert_eq!(
        provider
            .get_required_service::<automatic_context::Service>()
            .await
            .unwrap()
            .value,
        61
    );
    assert_eq!(
        provider
            .get_required_service::<automatic_instance::Service>()
            .await
            .unwrap()
            .value,
        67
    );
    let generic = provider
        .get_required_service::<generic_automatic::Service<generic_automatic::Dependency>>()
        .await
        .unwrap();
    assert_eq!(generic.value, 144);
    assert!(std::ptr::eq(
        &*generic.dependency,
        provider
            .get_required_service::<generic_automatic::Dependency>()
            .await
            .unwrap()
    ));
    assert_eq!(
        provider
            .get_required_service::<configured_automatic::Service>()
            .await
            .unwrap()
            .value,
        198
    );
    let explicit = provider
        .get_required_service::<generic_constructor::Service<generic_automatic::Dependency>>()
        .await
        .unwrap();
    assert_eq!(explicit.value, 319);
    assert!(std::ptr::eq(&*explicit.dependency, &*generic.dependency));
    assert_eq!(
        provider
            .get_required_service::<static_bindings::Automatic>()
            .await
            .unwrap()
            .value,
        276
    );
    assert_eq!(
        provider
            .get_required_service::<static_bindings::Explicit>()
            .await
            .unwrap()
            .value,
        371
    );
    provider.dispose_async().await.unwrap();
}

#[tokio::test]
async fn raw_identifiers_preserve_business_references_for_types_factories_and_constructors() {
    let provider = ServiceProvider::build(None).await.unwrap();
    let dependency = provider
        .get_required_service::<raw_injectable::r#type>()
        .await
        .unwrap();
    let generic = provider
        .get_required_service::<raw_injectable::r#struct<raw_injectable::r#type>>()
        .await
        .unwrap();
    assert_eq!(generic.r#type, 79);
    assert!(std::ptr::eq(&*generic.dependency, dependency));
    assert_eq!(
        provider
            .get_required_service::<raw_factory::Sync>()
            .await
            .unwrap()
            .0,
        83
    );
    assert_eq!(
        provider
            .get_required_service::<raw_factory::Async>()
            .await
            .unwrap()
            .0,
        85
    );
    assert_eq!(
        provider
            .get_required_service::<raw_factory::Future>()
            .await
            .unwrap()
            .0,
        87
    );
    let failure = match provider
        .get_required_service::<raw_factory::Failure>()
        .await
    {
        Ok(_) => panic!("raw factory must retain its error"),
        Err(error) => error,
    };
    assert!(failure.to_string().contains("raw factory failure"));
    assert_eq!(
        provider
            .get_required_service::<raw_constructor::Service>()
            .await
            .unwrap()
            .value,
        89
    );
    provider.dispose_async().await.unwrap();
}
