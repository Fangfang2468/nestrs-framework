use super::*;
use crate::registration::provider::Provider;

struct Defined<T>(PhantomData<T>);
impl<T: Send + Sync + 'static> ProviderDefinition for Defined<T> {
    fn provider() -> Provider {
        panic!("discovering a callback must never execute it")
    }
}
struct FactoryOnly;
struct GenericFactoryOnly<T>(PhantomData<T>);
trait Port: Send + Sync {}

#[test]
fn concrete_site_probe_finds_definitions_through_type_aliases() {
    type Alias = Defined<u32>;
    assert!(
        (&&Probe::<Defined<u32>>::new())
            .provider_callback()
            .is_some()
    );
    assert!((&&Probe::<Alias>::new()).provider_callback().is_some());
}

#[test]
#[allow(clippy::needless_borrow)] // 保留宏在 fallback 类型处实际使用的 autoref 协议。
fn factory_only_generic_and_trait_queries_use_the_unbounded_fallback() {
    type FactoryAlias = GenericFactoryOnly<u32>;
    type TraitAlias = dyn Port;
    assert!(
        (&&Probe::<FactoryOnly>::new())
            .provider_callback()
            .is_none()
    );
    assert!(
        (&&Probe::<GenericFactoryOnly<u32>>::new())
            .provider_callback()
            .is_none()
    );
    assert!(
        (&&Probe::<FactoryAlias>::new())
            .provider_callback()
            .is_none()
    );
    assert!((&&Probe::<dyn Port>::new()).provider_callback().is_none());
    assert!((&&Probe::<TraitAlias>::new()).provider_callback().is_none());
}
