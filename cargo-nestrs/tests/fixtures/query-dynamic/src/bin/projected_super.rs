//! Each route uses a distinct service type so a static control cannot hide a lost dyn root.
use dynamic_library::{self as library, projected};
use nestrs::injectable;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static LOCAL_BUILDS: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct Repository<T: Send + Sync + 'static> {
    #[value(LOCAL_BUILDS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    marker: PhantomData<T>,
}
trait Map {
    type Target: Send + Sync + 'static;
}
trait Run<T> {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> library::Query<'a>;
}
trait Child<M: Map>: Run<M::Target> {}
struct Runner<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> Run<T> for Runner<T> {
    fn run<'a>(&self, provider: &'a ServiceProvider) -> library::Query<'a> {
        Box::pin(async move {
            let repository = provider
                .get_required_service::<Repository<T>>()
                .await
                .unwrap();
            assert!(repository.id < 3);
        })
    }
}
impl<M: Map> Child<M> for Runner<M::Target> {}

struct StaticTag;
struct StaticMap;
struct ParentTag;
struct ProjectedTag;
struct LocalMap;
impl Map for StaticMap {
    type Target = StaticTag;
}
impl Map for LocalMap {
    type Target = ProjectedTag;
}

struct CrossTag;
struct NestedTag;
struct DefaultTag;
struct RejectedTag;
struct CrossMap;
struct InnerMap;
struct OuterMap;
struct DefaultMap;
struct RejectedMap;
impl projected::Map for CrossMap {
    type Target = CrossTag;
}
impl projected::Map for InnerMap {
    type Target = NestedTag;
}
impl projected::Map for OuterMap {
    type Target = InnerMap;
}
impl projected::Map for DefaultMap {
    type Target = DefaultTag;
}
impl projected::Map for RejectedMap {
    type Target = RejectedTag;
}

#[cfg(feature = "invalid-projected")]
struct InvalidTag;
#[cfg(feature = "invalid-projected")]
struct InvalidMap;
#[cfg(feature = "invalid-projected")]
impl projected::Map for InvalidMap {
    type Target = InvalidTag;
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build(Some(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    }))
    .await
    .unwrap();
    assert_eq!(LOCAL_BUILDS.load(Ordering::SeqCst), 3);
    assert_eq!(library::builds(), 3);
    assert_eq!(library::queries(), 0);

    <Runner<StaticTag> as Run<<StaticMap as Map>::Target>>::run(
        &Runner::<StaticTag>(PhantomData),
        &provider,
    )
    .await;
    let direct_parent: Box<dyn Run<ParentTag>> = Box::new(Runner::<ParentTag>(PhantomData));
    direct_parent.run(&provider).await;
    let projected_child: Box<dyn Child<LocalMap>> = Box::new(Runner::<ProjectedTag>(PhantomData));
    projected_child.run(&provider).await;

    // The same Child/Run definitions with a different projected parameter cannot
    // make this unused invalid implementation a candidate for CrossMap's call.
    let _rejected = projected::erase_rejected::<RejectedMap>();
    projected::erase::<CrossMap>().projected(&provider).await;
    projected::erase_nested::<OuterMap>()
        .projected(&provider)
        .await;
    projected::erase_default::<DefaultMap>()
        .projected_default(&provider)
        .await;
    assert_eq!(library::queries(), 3);

    #[cfg(feature = "invalid-projected")]
    if false {
        projected::erase_invalid::<InvalidMap>()
            .projected_invalid(&provider)
            .await;
    }
    provider.dispose_async().await.unwrap();
    println!("projected supertrait query contracts passed");
}
