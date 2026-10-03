//! Distinct tags prevent a direct control from seeding a lost external query.
#![allow(dead_code)]
use nestrs::injectable;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use plain_helper::Run;
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static BUILDS: AtomicUsize = AtomicUsize::new(0);
static QUERIES: AtomicUsize = AtomicUsize::new(0);

#[injectable]
struct Repository<T: Send + Sync + 'static> {
    #[value(BUILDS.fetch_add(1, Ordering::SeqCst))]
    id: usize,
    marker: PhantomData<T>,
}
struct Runner<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> Run<ServiceProvider> for Runner<T> {
    fn run(&self, provider: &ServiceProvider) {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let value = provider
                    .get_required_service::<Repository<T>>()
                    .await
                    .unwrap_or_else(|error| panic!("{}: {error:?}", std::any::type_name::<T>()));
                assert!(value.id < 16);
                QUERIES.fetch_add(1, Ordering::SeqCst);
            })
        });
    }
}
struct LocalProvider<'a, T>(&'a ServiceProvider, PhantomData<T>);
impl<T: Send + Sync + 'static> Run<LocalProvider<'_, T>> for u64 {
    fn run(&self, provider: &LocalProvider<'_, T>) {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let value = provider
                    .0
                    .get_required_service::<Repository<T>>()
                    .await
                    .unwrap();
                assert!(value.id < 16);
                QUERIES.fetch_add(1, Ordering::SeqCst);
            })
        });
    }
}
struct InvalidLocalProvider<'a, T>(&'a ServiceProvider, PhantomData<T>);
impl<T: Send + Sync + 'static> Run<InvalidLocalProvider<'_, T>> for u64 {
    fn run(&self, provider: &InvalidLocalProvider<'_, T>) {
        drop(provider.0.get_required_service::<InvalidRepository<T>>());
    }
}
struct FamilyMaker<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> plain_helper::Family<ServiceProvider> for FamilyMaker<T> {
    type Target = Runner<T>;
}
struct NestedFamilyMaker<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> plain_helper::NestedFamily<ServiceProvider>
    for NestedFamilyMaker<T>
{
    type Inner = FamilyMaker<T>;
}
struct InvalidFamilyMaker<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> plain_helper::Family<ServiceProvider> for InvalidFamilyMaker<T> {
    type Target = InvalidRunner<T>;
}
struct InvalidNestedFamilyMaker<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> plain_helper::NestedFamily<ServiceProvider>
    for InvalidNestedFamilyMaker<T>
{
    type Inner = InvalidFamilyMaker<T>;
}
impl<T> Default for Runner<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
impl<T> Default for InvalidRunner<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}
struct FamilyTag;
struct FamilyDefaultTag;
struct FamilyReturnTag;
struct FamilyNestedTag;
struct FamilyDynamicTag;
struct PrimitiveTag;
struct PrimitiveDirectTag;
struct DirectTag;
struct ForwardTag;
struct ErasureTag;
struct DeepTag;
struct ClosureTag;
struct DefaultTag;
struct AssociatedTag;
struct UnexecutedTag;
struct UnexecutedErasureTag;
struct UnexecutedAssociatedTag;
struct BadTag;
struct Unregistered;

#[injectable]
struct InvalidRepository<T: Send + Sync + 'static> {
    #[inject]
    missing: Unregistered,
    marker: PhantomData<T>,
}
struct InvalidRunner<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> Run<ServiceProvider> for InvalidRunner<T> {
    fn run(&self, provider: &ServiceProvider) {
        drop(provider.get_required_service::<InvalidRepository<T>>());
    }
}

#[tokio::main]
async fn main() {
    let provider = ServiceProvider::build(Some(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    }))
    .await
    .unwrap();
    Runner::<DirectTag>(PhantomData).run(&provider);
    #[cfg(not(feature = "direct-only"))]
    {
        plain_helper::invoke(&Runner::<ForwardTag>(PhantomData), &provider);
        plain_helper::invoke(
            &5u64,
            &LocalProvider::<PrimitiveTag>(&provider, PhantomData),
        );
        plain_helper::erase(Runner::<ErasureTag>(PhantomData)).run(&provider);
        plain_relay::invoke(&Runner::<DeepTag>(PhantomData), &provider);
        plain_helper::invoke_closure(&Runner::<ClosureTag>(PhantomData), &provider);
        plain_helper::invoke_default(&Runner::<DefaultTag>(PhantomData), &provider);
        plain_helper::invoke_associated(&Runner::<AssociatedTag>(PhantomData), &provider);
        plain_helper::invoke_unexecuted(&Runner::<UnexecutedTag>(PhantomData), &provider);
        plain_helper::invoke_unexecuted_erasure(
            Runner::<UnexecutedErasureTag>(PhantomData),
            &provider,
        );
        plain_helper::invoke_unexecuted_associated(
            &Runner::<UnexecutedAssociatedTag>(PhantomData),
            &provider,
        );
        plain_helper::invoke_family::<FamilyMaker<FamilyTag>, _>(&Runner(PhantomData), &provider);
        plain_helper::invoke_family_default::<FamilyMaker<FamilyDefaultTag>, _>(&provider);
        drop(plain_helper::invoke_family_return::<
            FamilyMaker<FamilyReturnTag>,
            _,
        >(&provider));
        plain_helper::invoke_family_nested::<NestedFamilyMaker<FamilyNestedTag>, _>(&provider);
        plain_helper::invoke_family_dynamic::<FamilyMaker<FamilyDynamicTag>, _>(&provider);
        assert_eq!(
            BUILDS.load(Ordering::SeqCst),
            16,
            "even unexecuted compiled queries are roots"
        );
        assert_eq!(QUERIES.load(Ordering::SeqCst), 13);
    }
    #[cfg(feature = "direct-only")]
    {
        #[cfg(feature = "primitive-direct")]
        <u64 as Run<LocalProvider<'_, PrimitiveDirectTag>>>::run(
            &5u64,
            &LocalProvider(&provider, PhantomData),
        );
        let expected = if cfg!(feature = "primitive-direct") {
            2
        } else {
            1
        };
        assert_eq!(BUILDS.load(Ordering::SeqCst), expected);
        assert_eq!(QUERIES.load(Ordering::SeqCst), expected);
    }
    #[cfg(feature = "invalid-family")]
    plain_helper::invoke_family::<InvalidFamilyMaker<BadTag>, _>(
        &InvalidRunner(PhantomData),
        &provider,
    );
    #[cfg(feature = "invalid-family-default")]
    plain_helper::invoke_family_default::<InvalidFamilyMaker<BadTag>, _>(&provider);
    #[cfg(feature = "invalid-family-return")]
    drop(plain_helper::invoke_family_return::<
        InvalidFamilyMaker<BadTag>,
        _,
    >(&provider));
    #[cfg(feature = "invalid-family-nested")]
    plain_helper::invoke_family_nested::<InvalidNestedFamilyMaker<BadTag>, _>(&provider);
    #[cfg(feature = "invalid-family-dynamic")]
    plain_helper::invoke_family_dynamic::<InvalidFamilyMaker<BadTag>, _>(&provider);
    #[cfg(feature = "invalid-primitive")]
    plain_helper::invoke(
        &5u64,
        &InvalidLocalProvider::<BadTag>(&provider, PhantomData),
    );
    #[cfg(feature = "invalid-forward")]
    plain_helper::invoke(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-erasure")]
    plain_helper::erase(InvalidRunner::<BadTag>(PhantomData)).run(&provider);
    #[cfg(feature = "invalid-deep")]
    plain_relay::invoke(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-closure")]
    plain_helper::invoke_closure(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-default")]
    plain_helper::invoke_default(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-associated")]
    plain_helper::invoke_associated(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-unexecuted")]
    plain_helper::invoke_unexecuted(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-unexecuted-erasure")]
    plain_helper::invoke_unexecuted_erasure(InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-unexecuted-associated")]
    plain_helper::invoke_unexecuted_associated(&InvalidRunner::<BadTag>(PhantomData), &provider);
    #[cfg(feature = "invalid-direct")]
    InvalidRunner::<BadTag>(PhantomData).run(&provider);
    provider.dispose_async().await.unwrap();
    println!("external query contracts passed");
}
