use std::marker::PhantomData;

use nestrs_core::__private::{
    ClassProvider, ConstructionContext, Provider, ProviderDefinition, ProviderSource,
    REFLECTED_PROVIDERS, ServiceIdentifier, ServiceLifetime, ServiceType,
};
use nestrs_macro::injectable;

struct User;
struct Entity;

// 这三个类型对应嵌套闭合泛型链：C -> B<u32> -> A<u32>。
// 它们刻意都使用本 crate 的类型，以便 #[injectable] 可以为开放泛型生成
// ProviderDefinition；当前分支尚无 ServiceProvider runtime，本测试只验证可递归
// 物化的 provider metadata。
#[injectable]
struct A<T> {
    marker: PhantomData<T>,
}

#[injectable]
struct B<T> {
    #[inject]
    a: A<T>,
}

#[injectable]
struct C {
    #[inject]
    b: B<u32>,
}

async fn cleanup_repository() {}

/// 泛型 injectable 本身不应向 linkme 写入一个开放类型的 provider；具体类型的
/// provider 由 `ProviderDefinition` 在依赖使用处按需物化。
#[injectable(lifetime = Transient, cleanup = "cleanup_repository")]
struct Repository<T> {
    #[value("generic-repository")]
    label: String,
    marker: PhantomData<T>,
}

#[injectable]
struct UserService {
    #[inject]
    repository: Repository<User>,
}

#[test]
fn generic_injectable_materializes_concrete_provider_definitions() {
    let direct = <Repository<Entity> as ProviderDefinition>::provider();
    let Provider::Class(ClassProvider {
        provide,
        common,
        dependencies,
        constructor,
        ..
    }) = direct
    else {
        panic!("generic provider definition should produce Provider::Class");
    };

    assert_eq!(
        provide,
        ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>())
    );
    assert_eq!(common.lifetime, ServiceLifetime::Transient);
    assert!(dependencies.is_empty());
    let cleanup = common
        .cleanup
        .expect("generic provider should retain its cleanup hook");
    drop(cleanup());

    let erased_direct_repository = constructor(ConstructionContext::new())
        .expect("generic Repository<Entity> constructor should not need dependencies");
    let direct_repository = match erased_direct_repository.downcast::<Repository<Entity>>() {
        Ok(repository) => repository,
        Err(_) => panic!("generic provider definition should retain its concrete type"),
    };
    assert_eq!(direct_repository.label, "generic-repository");
}

#[test]
fn injected_generic_repository_exposes_a_closed_provider_callback() {
    let providers: Vec<_> = REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .collect();
    let user_service = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class(ClassProvider { provide, .. })
                    if provide.service_type == ServiceType::create::<UserService>()
            )
        })
        .expect("non-generic UserService should be collected by linkme");

    // `Repository<T>` is an open generic definition, so it must not register an arbitrary
    // `Repository<Entity>` or `Repository<User>` provider eagerly.
    assert!(!providers.iter().any(|provider| {
        matches!(
            provider,
            Provider::Class(ClassProvider { provide, .. })
                if provide.service_type == ServiceType::create::<Repository<Entity>>()
                    || provide.service_type == ServiceType::create::<Repository<User>>()
        )
    }));

    let Provider::Class(ClassProvider { dependencies, .. }) = user_service else {
        panic!("UserService should be a class provider");
    };
    let dependency = dependencies
        .first()
        .expect("UserService should describe its Repository<User> dependency");
    assert_eq!(dependency.declaration_position, 0);
    assert_eq!(dependency.input_position.0, 0);
    assert_eq!(
        dependency.token,
        ServiceIdentifier::from(ServiceType::create::<Repository<User>>())
    );

    let repository_provider = match dependency.provider_source {
        ProviderSource::Materialize(definition) => definition(),
        ProviderSource::Registered => {
            panic!("generic injection should carry its closed provider callback")
        }
    };
    let Provider::Class(ClassProvider {
        provide,
        constructor,
        ..
    }) = repository_provider
    else {
        panic!("generic callback should produce Provider::Class");
    };
    assert_eq!(
        provide,
        ServiceIdentifier::from(ServiceType::create::<Repository<User>>())
    );

    let erased_repository = constructor(ConstructionContext::new())
        .expect("Repository<User> callback should construct the concrete service");
    let repository = match erased_repository.downcast::<Repository<User>>() {
        Ok(repository) => repository,
        Err(_) => panic!("generic dependency callback should retain Repository<User>"),
    };
    assert_eq!(repository.label, "generic-repository");
}

#[test]
fn nested_closed_generics_materialize_a_complete_descriptor_chain() {
    let providers: Vec<_> = REFLECTED_PROVIDERS
        .iter()
        .map(|provider| provider())
        .collect();

    println!(
        "--------------------------------   nested_closed_generics_materialize_a_complete_descriptor_chain   --------------------------------"
    );
    println!("{providers:#?}");

    let c_provider = providers
        .iter()
        .find(|provider| {
            matches!(
                provider,
                Provider::Class(ClassProvider { provide, .. })
                    if provide.service_type == ServiceType::create::<C>()
            )
        })
        .expect("C should be registered through linkme");

    assert!(!providers.iter().any(|provider| {
        matches!(
            provider,
            Provider::Class(ClassProvider { provide, .. })
                if provide.service_type == ServiceType::create::<B<u32>>()
                    || provide.service_type == ServiceType::create::<A<u32>>()
        )
    }));

    let Provider::Class(ClassProvider { dependencies, .. }) = c_provider else {
        panic!("C should be described by a class provider")
    };
    assert_eq!(dependencies.len(), 1);
    let b_dependency = dependencies
        .first()
        .expect("C should declare its B<u32> dependency");
    assert_eq!(
        b_dependency.token,
        ServiceIdentifier::from(ServiceType::create::<B<u32>>())
    );
    let b_provider = match b_dependency.provider_source {
        ProviderSource::Materialize(definition) => definition(),
        ProviderSource::Registered => {
            panic!("B<u32> should carry a closed generic provider callback")
        }
    };

    let Provider::Class(ClassProvider {
        provide,
        dependencies,
        ..
    }) = &b_provider
    else {
        panic!("B<u32> callback should produce a class provider")
    };
    assert_eq!(
        *provide,
        ServiceIdentifier::from(ServiceType::create::<B<u32>>())
    );
    assert_eq!(dependencies.len(), 1);
    let a_dependency = dependencies
        .first()
        .expect("B<u32> should declare its A<u32> dependency");
    assert_eq!(
        a_dependency.token,
        ServiceIdentifier::from(ServiceType::create::<A<u32>>())
    );
    let a_provider = match a_dependency.provider_source {
        ProviderSource::Materialize(definition) => definition(),
        ProviderSource::Registered => {
            panic!("A<u32> should carry a closed generic provider callback")
        }
    };

    let Provider::Class(ClassProvider {
        provide,
        dependencies,
        constructor,
        ..
    }) = a_provider
    else {
        panic!("A<u32> callback should produce a class provider")
    };
    assert_eq!(
        provide,
        ServiceIdentifier::from(ServiceType::create::<A<u32>>())
    );
    assert!(dependencies.is_empty());

    let erased_a = constructor(ConstructionContext::new())
        .expect("the leaf closed generic should construct without dependencies");
    assert!(erased_a.downcast::<A<u32>>().is_ok());
}
