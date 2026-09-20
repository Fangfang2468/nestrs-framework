use std::marker::PhantomData;

use nestrs_core::__private::{
    ClassProvider, ConstructionContext, Provider, ProviderDefinition, ProviderSource,
    REFLECTED_PROVIDERS, ServiceIdentifier, ServiceType,
};
use nestrs_macro::injectable;

// 此测试 crate 中只声明这两个开放泛型服务。
#[injectable]
struct A<T> {
    marker: PhantomData<T>,
}

#[injectable]
struct B<T> {
    #[inject]
    a: A<T>,
}

#[test]
fn open_generic_chain_is_not_eagerly_registered_without_a_closed_root() {
    let providers: Vec<_> = REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .collect();

    // 开放泛型没有可注册到 linkme 的具体 TypeId；没有 C 这类闭合根服务时，
    // 静态 provider 切片必须为空。
    assert!(providers.is_empty());

    // 不依赖根服务也可以显式验证单态化后的 provider definition。它自身不构造
    // B<u32>，而是声明 A<u32> 的按需物化依赖。
    let b_provider = <B<u32> as ProviderDefinition>::provider();

    println!("-----------------------------  b_provider  -----------------------------");
    println!("{b_provider:#?}");

    let Provider::Class(ClassProvider {
        provide,
        dependencies,
        ..
    }) = b_provider
    else {
        panic!("B<u32> provider definition should produce Provider::Class");
    };
    assert_eq!(
        provide,
        ServiceIdentifier::from(ServiceType::create::<B<u32>>())
    );
    assert_eq!(dependencies.len(), 1);

    let a_dependency = &dependencies[0];
    assert_eq!(
        a_dependency.token,
        ServiceIdentifier::from(ServiceType::create::<A<u32>>())
    );
    let a_provider = match a_dependency.provider_source {
        ProviderSource::Materialize(definition) => definition(),
        ProviderSource::Registered => {
            panic!("A<u32> should be materialized from B<u32>'s dependency")
        }
    };

    let Provider::Class(ClassProvider {
        provide,
        dependencies,
        constructor,
        ..
    }) = a_provider
    else {
        panic!("A<u32> provider definition should produce Provider::Class");
    };
    assert_eq!(
        provide,
        ServiceIdentifier::from(ServiceType::create::<A<u32>>())
    );
    assert!(dependencies.is_empty());

    let erased_a = constructor(ConstructionContext::new())
        .expect("A<u32> should construct without injected dependencies");
    assert!(erased_a.downcast::<A<u32>>().is_ok());
}
