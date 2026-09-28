use nestrs::{factory, injectable};
use nestrs_core::{
    __private::{Provider, REFLECTED_PROVIDERS, REFLECTED_ROOTS, ServiceKey, ServiceType},
    ServiceProvider, get_required_service, get_service,
};

use std::marker::PhantomData;

#[injectable(key = "root")]
struct Repository<T> {
    marker: PhantomData<T>,
}

struct User;
struct Order;
struct Missing;
struct FactoryOnly<T> {
    marker: PhantomData<T>,
}
trait Port: Send + Sync {}

type UserRepository = Repository<User>;
type PortAlias = dyn Port;

#[factory]
fn factory_only() -> FactoryOnly<User> {
    FactoryOnly {
        marker: PhantomData,
    }
}

// A compiled query site contributes a static root independently of function execution.
#[allow(dead_code)]
fn never_executed_queries(provider: &ServiceProvider) {
    drop(get_service!(provider, UserRepository));
    drop(get_required_service!(provider, Repository<Order>));
    if false {
        drop(get_service!(provider, Repository<User>));
    }
    drop(get_service!(provider, FactoryOnly<User>));
    drop(get_service!(provider, PortAlias));
    drop(get_service!(provider, Missing));
    #[cfg(any())]
    drop(get_service!(provider, ThisTypeDoesNotExist));
}

#[test]
fn query_macros_collect_closed_types_aliases_and_fallback_roots_before_execution() {
    // Only the ordinary factory is in this slice; open generic declarations stay lazy.
    assert_eq!(REFLECTED_PROVIDERS.len(), 1);
    let roots: Vec<_> = REFLECTED_ROOTS.iter().map(|declare| declare()).collect();
    assert_eq!(roots.len(), 6);
    assert_eq!(
        roots
            .iter()
            .filter(|root| root.service_type == ServiceType::create::<Repository<User>>())
            .count(),
        2
    );
    assert_eq!(
        roots
            .iter()
            .filter(|root| root.materialize.is_some())
            .count(),
        3
    );
    for root in roots {
        if let Some(materialize) = root.materialize {
            let Provider::Class(provider) = materialize() else {
                panic!("generic class root")
            };
            assert_eq!(provider.provide.service_type, root.service_type);
            assert_eq!(
                provider.provide.service_key,
                Some(ServiceKey::Named("root".into()))
            );
        } else {
            assert!(
                [
                    ServiceType::create::<FactoryOnly<User>>(),
                    ServiceType::create::<dyn Port>(),
                    ServiceType::create::<Missing>(),
                ]
                .contains(&root.service_type)
            );
        }
        assert!(root.source.file.ends_with("registered_roots.rs"));
    }
}
