//! Expansion-order and source-context regressions for the shared declaration frontend.
use nestrs::injectable;
use nestrs_core::{Injection, ServiceProvider};

use std::sync::atomic::Ordering;

#[path = "runtime_lowering/external.rs"]
mod external;

mod generated {
    use nestrs::injectable;
    macro_rules! declare_component {
        ($declaration:path, $name:ident, $dependency:ty, $label:expr) => {
            #[$declaration]
            pub(super) struct $name {
                #[nestrs::inject]
                pub(super) database: $dependency,
                #[nestrs::value($label)]
                pub(super) label: String,
            }
        };
    }

    // The attribute path, dependency alias and value expression retain their caller context.
    declare_component!(
        injectable,
        Generated,
        super::external::DatabaseAlias,
        super::component_label()
    );
}

fn component_label() -> String {
    "generated-service".to_owned()
}

#[injectable]
#[derive(Default)]
struct DerivedConsumer {
    #[inject]
    database: Option<external::DatabaseAlias>,
}

#[cfg_attr(all(), injectable)]
struct EnabledByCfgAttr {
    #[value(31)]
    number: usize,
}

// These declarations must be removed before any Nestrs validation or type checking.
#[cfg(any())]
#[injectable]
enum DisabledInvalidComponent {
    Variant,
}

#[cfg(any())]
#[nestrs::factory]
unsafe fn disabled_invalid_factory() -> DoesNotExist {
    unimplemented!()
}

#[injectable]
struct Repository<T> {
    #[inject]
    dependency: T,
}

type RepositoryAlias = Repository<external::DatabaseAlias>;

#[injectable]
struct AliasConsumer {
    #[inject]
    repository: RepositoryAlias,
}

fn accepts_optional_token(_: &Option<Injection<external::Database>>) {}

#[tokio::test]
async fn declaration_lowering_preserves_expansion_context_and_factory_borrows() {
    let provider = ServiceProvider::build(None).await.unwrap();
    assert_eq!(external::CONSTRUCTIONS.load(Ordering::SeqCst), 0);

    let database = provider
        .get_required_service::<external::DatabaseAlias>()
        .await
        .unwrap();
    let generated = provider
        .get_required_service::<generated::Generated>()
        .await
        .unwrap();
    assert_eq!(generated.label, "generated-service");
    assert!(std::ptr::eq(&*generated.database, database));

    let default_consumer = DerivedConsumer::default();
    accepts_optional_token(&default_consumer.database);
    assert!(default_consumer.database.is_none());
    let derived = provider
        .get_required_service::<DerivedConsumer>()
        .await
        .unwrap();
    accepts_optional_token(&derived.database);
    assert!(std::ptr::eq(
        &**derived.database.as_ref().unwrap(),
        database
    ));

    assert_eq!(
        provider
            .get_required_service::<EnabledByCfgAttr>()
            .await
            .unwrap()
            .number,
        31
    );

    let consumer = provider
        .get_required_service::<AliasConsumer>()
        .await
        .unwrap();
    let repository = provider
        .get_required_service::<RepositoryAlias>()
        .await
        .unwrap();
    assert!(std::ptr::eq(&*consumer.repository, repository));
    assert!(std::ptr::eq(&*repository.dependency, database));

    let (sync, asynchronous, future) = tokio::join!(
        provider.get_required_service::<external::SyncSummary>(),
        provider.get_required_service::<external::AsyncSummary>(),
        provider.get_required_service::<external::FutureSummary>(),
    );
    assert_eq!(sync.unwrap().text, "orders-primary:sync");
    assert_eq!(asynchronous.unwrap().text, "orders-primary:async");
    assert_eq!(future.unwrap().text, "orders-primary:future");
    assert_eq!(external::CONSTRUCTIONS.load(Ordering::SeqCst), 1);
    assert_eq!(external::DROPS.load(Ordering::SeqCst), 0);

    provider.dispose_async().await.unwrap();
    assert_eq!(external::DROPS.load(Ordering::SeqCst), 1);
}
