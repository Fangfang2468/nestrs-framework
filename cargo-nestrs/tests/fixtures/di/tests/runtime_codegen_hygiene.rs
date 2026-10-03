//! Generated item and binding hygiene must preserve business names and side effects.
#![allow(non_camel_case_types, non_upper_case_globals)]

use nestrs_core::ServiceProvider;

mod expressions {
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub static CALLS: AtomicUsize = AtomicUsize::new(0);
    mod __nestrs_reflect {
        pub const VALUE: usize = 17;
        pub fn compiler_binding<C, I>() {
            super::CALLS.fetch_add(1, super::Ordering::SeqCst);
        }
    }
    fn __nestrs_construct() -> usize {
        19
    }
    fn __nestrs_reflect_provider() -> usize {
        23
    }

    #[nestrs::injectable]
    pub struct Service {
        #[cfg(any())]
        disabled: Missing,
        #[value(__nestrs_construct())]
        pub constructed: usize,
        #[value(__nestrs_reflect_provider())]
        pub reflected: usize,
        #[value(__nestrs_reflect::VALUE)]
        pub module_value: usize,
        // The internal marker has the same signature; a capture would compile
        // successfully but silently skip the business Atomic side effect.
        #[value(__nestrs_reflect::compiler_binding::<Service, Service>())]
        pub effect: (),
    }

    #[nestrs::injectable]
    pub struct Generic<T: Send + Sync + 'static> {
        #[value(__nestrs_construct())]
        pub value: usize,
        marker: std::marker::PhantomData<T>,
    }
}

mod marker_constants {
    pub const __nestrs_constructor_field_mode: usize = 29;
    pub const __nestrs_constructor_input: &str = "business input";

    #[nestrs::injectable]
    pub struct Service {
        #[value(__nestrs_constructor_field_mode)]
        pub value: usize,
        #[value(__nestrs_constructor_input)]
        pub input: &'static str,
    }

    #[nestrs::injectable]
    pub struct Generic<T: Default + Send + Sync + 'static> {
        pub value: T,
    }
}

mod business_types {
    #[nestrs::injectable]
    pub struct __nestrs_reflect;
    #[nestrs::injectable]
    pub struct __nestrs_construct(#[value(31)] pub usize);
    #[nestrs::injectable]
    pub struct __nestrs_reflect_provider(#[value(37)] pub usize);

    #[nestrs::injectable]
    pub struct Generic<__nestrs_reflect: Send + Sync + 'static> {
        #[inject]
        pub dependency: __nestrs_reflect,
    }
}

mod constructors {
    #[nestrs::injectable]
    pub struct Dependency;
    #[nestrs::injectable]
    pub struct Service {
        pub dependency: Dependency,
        pub value: usize,
    }
    impl Service {
        pub const __NESTRS_CONSTRUCTOR: usize = 41;
        pub fn __nestrs_constructor_activate() -> usize {
            43
        }
        pub fn __nestrs_constructor_dependencies() -> usize {
            47
        }
        #[nestrs::constructor]
        fn new(dependency: Dependency) -> Self {
            Self {
                dependency,
                value: Self::__NESTRS_CONSTRUCTOR
                    + Self::__nestrs_constructor_activate()
                    + Self::__nestrs_constructor_dependencies(),
            }
        }
    }

    #[nestrs::injectable]
    pub struct OwnName {
        pub value: usize,
    }
    impl OwnName {
        #[nestrs::constructor]
        pub fn __nestrs_constructor_activate() -> Self {
            Self { value: 53 }
        }
    }

    macro_rules! declare {
        ($service:ident, $value:expr) => {
            #[nestrs::injectable]
            pub struct $service<T: Send + Sync + 'static> {
                pub dependency: T,
                pub value: usize,
            }
            impl<U: Send + Sync + 'static> $service<U> {
                pub const __NESTRS_CONSTRUCTOR: usize = $value;
                pub fn __nestrs_constructor_activate() -> usize {
                    $value + 1
                }
                pub fn __nestrs_constructor_dependencies() -> usize {
                    $value + 2
                }
                #[nestrs::constructor]
                fn r#new(dependency: U) -> Result<Self, &'static str> {
                    Ok(Self {
                        dependency,
                        value: Self::__NESTRS_CONSTRUCTOR
                            + Self::__nestrs_constructor_activate()
                            + Self::__nestrs_constructor_dependencies(),
                    })
                }
            }
        };
    }
    declare!(First, 59);
    declare!(r#type, 67);
}

mod factory_and_cleanup {
    use std::sync::atomic::{AtomicUsize, Ordering};
    pub static CLEANED: AtomicUsize = AtomicUsize::new(0);
    pub struct __nestrs_reflected_factory(pub usize);
    #[nestrs::factory]
    fn __nestrs_factory_construct() -> __nestrs_reflected_factory {
        __nestrs_reflected_factory(71)
    }
    pub struct Product(pub usize);
    #[nestrs::factory]
    async fn make(value: __nestrs_reflected_factory) -> Product {
        Product(value.0 + 2)
    }
    async fn __nestrs_reflect_provider() {
        CLEANED.fetch_add(1, Ordering::SeqCst);
    }
    #[nestrs::injectable(cleanup = "__nestrs_reflect_provider")]
    pub struct Cleaned;
}

mod explicit_binding {
    const service: usize = 73;
    const projected: usize = 79;
    const slot: usize = 83;
    const input: usize = 89;
    const target: usize = 97;
    pub trait Port: Send + Sync {
        fn value(&self) -> usize;
    }
    #[nestrs::injectable]
    pub struct __nestrs_reflect;
    #[nestrs::bind]
    impl Port for __nestrs_reflect {
        fn value(&self) -> usize {
            service + projected + slot + input + target
        }
    }
}

#[tokio::test]
async fn business_expressions_and_type_names_keep_their_original_resolution() {
    let root = ServiceProvider::build().await.unwrap();
    let service = root
        .get_required_service::<expressions::Service>()
        .await
        .unwrap();
    assert_eq!(service.constructed, 19);
    assert_eq!(service.reflected, 23);
    assert_eq!(service.module_value, 17);
    assert_eq!(service.effect, ());
    assert_eq!(
        expressions::CALLS.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(
        root.get_required_service::<expressions::Generic<u8>>()
            .await
            .unwrap()
            .value,
        19
    );
    let marker = root
        .get_required_service::<marker_constants::Service>()
        .await
        .unwrap();
    assert_eq!(marker.value, 29);
    assert_eq!(marker.input, "business input");
    assert_eq!(
        root.get_required_service::<marker_constants::Generic<usize>>()
            .await
            .unwrap()
            .value,
        0
    );
    assert_eq!(
        root.get_required_service::<business_types::__nestrs_construct>()
            .await
            .unwrap()
            .0,
        31
    );
    assert_eq!(
        root.get_required_service::<business_types::__nestrs_reflect_provider>()
            .await
            .unwrap()
            .0,
        37
    );
    let generic = root
        .get_required_service::<business_types::Generic<business_types::__nestrs_reflect>>()
        .await
        .unwrap();
    assert!(std::ptr::eq(
        &*generic.dependency,
        root.get_required_service::<business_types::__nestrs_reflect>()
            .await
            .unwrap()
    ));
    root.dispose_async().await.unwrap();
}

#[tokio::test]
async fn constructor_business_members_and_generated_helpers_have_distinct_identities() {
    let root = ServiceProvider::build().await.unwrap();
    let service = root
        .get_required_service::<constructors::Service>()
        .await
        .unwrap();
    assert_eq!(service.value, 131);
    assert_eq!(constructors::Service::__NESTRS_CONSTRUCTOR, 41);
    assert_eq!(constructors::Service::__nestrs_constructor_activate(), 43);
    assert_eq!(
        constructors::Service::__nestrs_constructor_dependencies(),
        47
    );
    assert!(std::ptr::eq(
        &*service.dependency,
        root.get_required_service::<constructors::Dependency>()
            .await
            .unwrap()
    ));
    assert_eq!(
        constructors::OwnName::__nestrs_constructor_activate().value,
        53
    );
    assert_eq!(
        root.get_required_service::<constructors::OwnName>()
            .await
            .unwrap()
            .value,
        53
    );
    let first = root
        .get_required_service::<constructors::First<constructors::Dependency>>()
        .await
        .unwrap();
    let second = root
        .get_required_service::<constructors::r#type<constructors::Dependency>>()
        .await
        .unwrap();
    assert_eq!(first.value, 180);
    assert_eq!(second.value, 204);
    assert!(std::ptr::eq(&*first.dependency, &*second.dependency));
    root.dispose_async().await.unwrap();
}

#[tokio::test]
async fn factory_cleanup_and_explicit_projection_preserve_business_names() {
    let root = ServiceProvider::build().await.unwrap();
    assert_eq!(
        root.get_required_service::<factory_and_cleanup::Product>()
            .await
            .unwrap()
            .0,
        73
    );
    let _ = root
        .get_required_service::<factory_and_cleanup::Cleaned>()
        .await
        .unwrap();
    let projected = root
        .get_required_service::<dyn explicit_binding::Port>()
        .await
        .unwrap();
    assert_eq!(projected.value(), 421);
    assert!(std::ptr::addr_eq(
        projected,
        root.get_required_service::<explicit_binding::__nestrs_reflect>()
            .await
            .unwrap()
    ));
    root.dispose_async().await.unwrap();
    assert_eq!(
        factory_and_cleanup::CLEANED.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}
