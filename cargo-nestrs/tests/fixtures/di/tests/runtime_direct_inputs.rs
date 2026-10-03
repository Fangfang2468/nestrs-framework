//! Direct parameter delivery preserves caller hygiene and business initialization order.
use nestrs::{constructor, injectable};
use nestrs_core::ServiceProvider;
use std::sync::Mutex;

static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

#[injectable]
struct Dependency;

// The generated input local intentionally has the same spelling. The value
// expression must continue to resolve this caller-owned function.
fn __nestrs_injectable_input_0() -> usize {
    EVENTS.lock().unwrap().push("value");
    17
}

struct DefaultValue;

impl Default for DefaultValue {
    fn default() -> Self {
        EVENTS.lock().unwrap().push("default");
        Self
    }
}

#[injectable]
struct Automatic {
    #[value(__nestrs_injectable_input_0())]
    first: usize,
    #[inject]
    dependency: Dependency,
    _default: DefaultValue,
    // Both closure argument inference and unsizing rely on the field context.
    #[value({ EVENTS.lock().unwrap().push("closure"); Box::new(|value| value + 2) })]
    transform: Box<dyn Fn(usize) -> usize + Send + Sync>,
    #[inject]
    optional: Option<Dependency>,
    #[value({ EVENTS.lock().unwrap().push("last"); 31 })]
    last: usize,
}

#[injectable]
struct Generic<T: Send + Sync + 'static> {
    #[inject]
    dependency: T,
    #[value(__nestrs_injectable_input_0())]
    value: usize,
}

#[injectable]
struct Explicit {
    dependency: Dependency,
    value: usize,
}

impl Explicit {
    #[constructor]
    fn new(dependency: Dependency, optional: Option<Dependency>) -> Self {
        assert!(optional.is_some());
        EVENTS.lock().unwrap().push("constructor");
        Self {
            dependency,
            value: 43,
        }
    }
}

mod constant_collision {
    use super::Dependency;
    use nestrs::{constructor, injectable};

    // mixed-site spans alone do not stop Rust from interpreting a new let
    // pattern as a caller-owned constant. No new binding may use these names.
    #[allow(non_upper_case_globals)]
    const __nestrs_injectable_input_0: usize = 59;
    #[allow(non_upper_case_globals)]
    const __nestrs_constructor_input_0: usize = 61;

    #[injectable]
    pub(super) struct Automatic<T: Send + Sync + 'static> {
        #[inject]
        pub(super) dependency: T,
        #[value(__nestrs_injectable_input_0)]
        pub(super) value: usize,
    }

    #[injectable]
    pub(super) struct Explicit {
        pub(super) dependency: Dependency,
        pub(super) value: usize,
    }

    impl Explicit {
        #[constructor]
        fn new(dependency: Dependency) -> Self {
            Self {
                dependency,
                value: __nestrs_constructor_input_0,
            }
        }
    }
}

#[tokio::test]
async fn generated_parameter_locals_preserve_hygiene_type_context_and_evaluation_order() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert!(EVENTS.lock().unwrap().is_empty());
    let automatic = provider.get_required_service::<Automatic>().await.unwrap();
    assert_eq!(automatic.first, 17);
    assert_eq!((automatic.transform)(5), 7);
    assert_eq!(automatic.last, 31);
    assert!(std::ptr::eq(
        &*automatic.dependency,
        &**automatic.optional.as_ref().unwrap()
    ));
    assert_eq!(
        *EVENTS.lock().unwrap(),
        ["value", "default", "closure", "last"]
    );

    let generic = provider
        .get_required_service::<Generic<Dependency>>()
        .await
        .unwrap();
    assert_eq!(generic.value, 17);
    assert!(std::ptr::eq(&*generic.dependency, &*automatic.dependency));
    let explicit = provider.get_required_service::<Explicit>().await.unwrap();
    assert_eq!(explicit.value, 43);
    assert!(std::ptr::eq(&*explicit.dependency, &*automatic.dependency));
    let constant_automatic = provider
        .get_required_service::<constant_collision::Automatic<Dependency>>()
        .await
        .unwrap();
    assert_eq!(constant_automatic.value, 59);
    assert!(std::ptr::eq(
        &*constant_automatic.dependency,
        &*automatic.dependency
    ));
    let constant_explicit = provider
        .get_required_service::<constant_collision::Explicit>()
        .await
        .unwrap();
    assert_eq!(constant_explicit.value, 61);
    assert!(std::ptr::eq(
        &*constant_explicit.dependency,
        &*automatic.dependency
    ));
    assert_eq!(
        *EVENTS.lock().unwrap(),
        [
            "value",
            "default",
            "closure",
            "last",
            "value",
            "constructor"
        ]
    );
    provider.dispose_async().await.unwrap();
}

mod factory_constant_collision {
    use nestrs::{factory, injectable};
    use nestrs_core::{LazyInjection, ServiceProvider};

    // These legal caller constants used to be interpreted as patterns in the
    // generated parameter locals and Result arms. Test both kinds of binding.
    #[allow(non_upper_case_globals)]
    const __nestrs_factory_input_0: usize = 67;
    #[allow(non_upper_case_globals)]
    const __nestrs_factory_input_1: usize = 71;
    #[allow(non_upper_case_globals)]
    const __nestrs_factory_context: usize = 11;
    #[allow(non_upper_case_globals)]
    const __nestrs_factory_service: usize = 73;
    #[allow(non_upper_case_globals)]
    const __nestrs_factory_error: &str = "expected factory failure";

    #[injectable]
    struct Value {
        #[value(11)]
        number: usize,
    }
    struct Missing;
    struct Sync(usize);
    struct Async(usize);
    struct Failed;
    struct Lazy {
        required: LazyInjection<Value>,
        optional: Option<LazyInjection<Value>>,
        absent: Option<LazyInjection<Missing>>,
        number: usize,
    }

    #[factory]
    fn synchronous(value: Value, optional: Option<Value>, absent: Option<Missing>) -> Sync {
        assert!(std::ptr::eq(value, optional.unwrap()));
        assert!(absent.is_none());
        Sync(value.number + __nestrs_factory_input_0)
    }

    #[factory]
    async fn asynchronous(
        value: Value,
        optional: Option<Value>,
        absent: Option<Missing>,
    ) -> Result<Async, &'static str> {
        let before = value.number;
        tokio::task::yield_now().await;
        assert_eq!(before, value.number);
        assert!(std::ptr::eq(value, optional.unwrap()));
        assert!(absent.is_none());
        Ok(Async(value.number + __nestrs_factory_input_1))
    }

    #[factory]
    fn failed(value: Value) -> Result<Failed, &'static str> {
        assert_eq!(value.number, __nestrs_factory_context);
        Err(__nestrs_factory_error)
    }

    #[factory]
    fn lazy(
        value: Value,
        #[lazy] required: Value,
        #[lazy] optional: Option<Value>,
        #[lazy] absent: Option<Missing>,
    ) -> impl Future<Output = Result<Lazy, &'static str>> {
        async move {
            tokio::task::yield_now().await;
            Ok(Lazy {
                required,
                optional,
                absent,
                number: value.number + __nestrs_factory_service,
            })
        }
    }

    pub(super) async fn verify(provider: &ServiceProvider) {
        let sync = provider.get_required_service::<Sync>().await.unwrap();
        assert_eq!(sync.0, 78);
        let asynchronous = provider.get_required_service::<Async>().await.unwrap();
        assert_eq!(asynchronous.0, 82);
        let lazy = provider.get_required_service::<Lazy>().await.unwrap();
        assert_eq!(lazy.number, 84);
        assert!(lazy.absent.is_none());
        let required = lazy.required.get().await.unwrap();
        let optional = lazy.optional.as_ref().unwrap().get().await.unwrap();
        let concrete = provider.get_required_service::<Value>().await.unwrap();
        assert_eq!(required.number, 11);
        assert!(std::ptr::eq(required, optional));
        assert!(std::ptr::eq(required, concrete));
        let failure = match provider.get_required_service::<Failed>().await {
            Ok(_) => panic!("the Result factory should preserve its failure"),
            Err(error) => error,
        };
        assert!(failure.to_string().contains(__nestrs_factory_error));
    }
}

mod factory_function_collision {
    use super::Dependency;
    use nestrs::factory;
    use nestrs_core::{LazyInjection, ServiceProvider};

    struct Sync;
    struct Async(LazyInjection<Dependency>);

    // The adapter reuses the function's name for its local context. The call
    // must still resolve the module function after that context becomes a tuple.
    #[factory]
    fn __nestrs_factory_context(_dependency: Dependency) -> Sync {
        Sync
    }

    #[factory]
    async fn __nestrs_factory_input_0(
        #[lazy] dependency: Dependency,
    ) -> Result<Async, &'static str> {
        tokio::task::yield_now().await;
        Ok(Async(dependency))
    }

    pub(super) async fn verify(provider: &ServiceProvider) {
        provider.get_required_service::<Sync>().await.unwrap();
        let asynchronous = provider.get_required_service::<Async>().await.unwrap();
        let dependency = provider.get_required_service::<Dependency>().await.unwrap();
        assert!(std::ptr::eq(
            asynchronous.0.get().await.unwrap(),
            dependency
        ));
    }
}

#[tokio::test]
async fn factory_input_tuples_preserve_caller_constants_and_function_names() {
    let provider = ServiceProvider::build(None).await.unwrap();
    factory_constant_collision::verify(&provider).await;
    factory_function_collision::verify(&provider).await;
    provider.dispose_async().await.unwrap();
}
