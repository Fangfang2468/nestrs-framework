//! 核心注册与构造 ABI 的内部契约回归。
//!
//! 用户 crate 无需访问注册内部类型；这些测试直接使用 owned 描述或图快照，
//! 保留原外部 ABI 用例的类型、输入槽位与强 lease 安全断言。

use crate::{
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, InputSlot, prepare_required,
    },
    lifetime::ServiceLifetime,
    registration::{
        dependency::{ClosedProviderCallback, Delivery, DependencyRequest, ProviderSource},
        provider::{
            ClassProvider, Provider, ProviderCommon, ProviderDefinition, provider_definition,
        },
    },
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};

mod class_provider {
    use crate::graph::GraphCompiler;

    use super::{
        ClassProvider, ConstructionError, ConstructionInputs, Delivery, DependencyRequest,
        ErasedService, InputSlot, Provider, ProviderCommon, ProviderSource, ServiceIdentifier,
        ServiceKey, ServiceLifetime, ServiceSource, ServiceType,
    };

    struct Component;
    struct Database;
    trait Audit: Send + Sync {}

    fn construct_component(
        context: ConstructionInputs,
    ) -> Result<ErasedService, ConstructionError> {
        context.ensure_all_consumed()?;
        Ok(ErasedService::new(Component))
    }

    fn component_provider() -> Provider {
        Provider::Class(ClassProvider {
            provide: ServiceIdentifier::new(
                Some(ServiceKey::Named("controller".to_owned())),
                ServiceType::create::<Component>(),
            ),
            common: ProviderCommon {
                lifetime: ServiceLifetime::Scoped,
                primary: true,
                lazy: None,
                source: ServiceSource::new("class_provider.rs", 30, 1),
                cleanup: None,
            },
            dependencies: vec![
                DependencyRequest {
                    declaration_position: 0,
                    input_slot: InputSlot::new(0),
                    label: Some("database"),
                    token: ServiceIdentifier::from(ServiceType::create::<Database>()),
                    optional: false,
                    lazy: None,
                    project: None,
                    delivery: Delivery::Direct(crate::activation::prepare_required::<Database>),
                    provider_source: ProviderSource::Registered,
                },
                DependencyRequest {
                    declaration_position: 2,
                    input_slot: InputSlot::new(1),
                    label: None,
                    token: ServiceIdentifier::new(
                        Some(ServiceKey::Indexed(7)),
                        ServiceType::create::<dyn Audit>(),
                    ),
                    optional: true,
                    lazy: None,
                    project: None,
                    delivery: Delivery::RequiresBindingOrAbsent(
                        crate::activation::prepare_optional_absent::<dyn Audit>,
                    ),
                    provider_source: ProviderSource::Registered,
                },
            ],
            constructor: construct_component,
        })
    }

    #[test]
    fn class_provider_keeps_provider_identity_and_dependency_input_layout() {
        let providers = [component_provider()];
        let provider = providers
            .iter()
            .find(|provider| {
                matches!(
                    provider,
                    Provider::Class(ClassProvider { provide, .. })
                        if provide.service_type == ServiceType::create::<Component>()
                )
            })
            .expect("owned registration should retain the class provider");

        let Provider::Class(ClassProvider {
            provide,
            common,
            dependencies,
            ..
        }) = provider
        else {
            panic!("selected registration should be a class provider")
        };

        assert_eq!(
            *provide,
            ServiceIdentifier::new(
                Some(ServiceKey::Named("controller".to_owned())),
                ServiceType::create::<Component>(),
            )
        );
        assert_eq!(common.lifetime, ServiceLifetime::Scoped);
        assert!(common.primary);
        assert!(common.cleanup.is_none());
        assert_eq!(dependencies.len(), 2);

        let required = &dependencies[0];
        assert_eq!(required.declaration_position, 0);
        assert_eq!(required.input_slot, InputSlot::new(0));
        assert_eq!(required.label, Some("database"));
        assert!(!required.optional);
        assert!(matches!(required.delivery, Delivery::Direct(_)));
        assert!(matches!(
            required.provider_source,
            ProviderSource::Registered
        ));

        let optional_tuple = &dependencies[1];
        assert_eq!(optional_tuple.declaration_position, 2);
        assert_eq!(optional_tuple.input_slot, InputSlot::new(1));
        assert_eq!(optional_tuple.label, None);
        assert!(optional_tuple.optional);
        assert!(matches!(
            optional_tuple.delivery,
            Delivery::RequiresBindingOrAbsent(_)
        ));
        assert_eq!(
            optional_tuple.token,
            ServiceIdentifier::new(
                Some(ServiceKey::Indexed(7)),
                ServiceType::create::<dyn Audit>(),
            )
        );

        // 描述存储与编译器的收集方式独立：将 owned 快照交给真实图编译器，
        // 验证同一服务的两个输入槽位仍分别保留。
        let database = Provider::Class(ClassProvider {
            provide: ServiceIdentifier::from(ServiceType::create::<Database>()),
            common: ProviderCommon {
                lifetime: ServiceLifetime::Singleton,
                primary: false,
                lazy: None,
                source: ServiceSource::new("class_provider.rs", 1, 1),
                cleanup: None,
            },
            dependencies: vec![],
            constructor: |_| panic!("graph compilation must not construct dependencies"),
        });
        let graph =
            GraphCompiler::compile_snapshot(crate::registration::catalog::RegistrySnapshot {
                providers: vec![database, provider.clone()],
                bindings: vec![],
                roots: vec![],
                ..Default::default()
            })
            .unwrap();
        let compiled = &graph.nodes[graph.routes[provide].provider];
        assert_eq!(compiled.common.lifetime, ServiceLifetime::Scoped);
        assert_eq!(compiled.common.source, common.source);
        assert_eq!(compiled.dependencies.len(), 2);
        assert_eq!(compiled.dependencies[0].slot, InputSlot::new(0));
        assert!(compiled.dependencies[0].target.is_some());
        assert_eq!(compiled.dependencies[1].slot, InputSlot::new(1));
        assert!(compiled.dependencies[1].optional);
        assert!(compiled.dependencies[1].target.is_none());
    }
}

mod provider_definition {
    use std::marker::PhantomData;

    use super::{
        ClassProvider, ClosedProviderCallback, ConstructionError, ConstructionInputs, Delivery,
        DependencyRequest, ErasedService, InputSlot, Provider, ProviderCommon, ProviderDefinition,
        ProviderSource, ServiceIdentifier, ServiceLifetime, ServiceSource, ServiceType,
        prepare_required, provider_definition,
    };

    struct Entity;
    struct Repository<T>(PhantomData<T>);

    fn construct_repository<T>(
        context: ConstructionInputs,
    ) -> Result<ErasedService, ConstructionError>
    where
        T: Send + Sync + 'static,
    {
        context.ensure_all_consumed()?;
        Ok(ErasedService::new(Repository::<T>(PhantomData)))
    }

    impl<T> ProviderDefinition for Repository<T>
    where
        T: Send + Sync + 'static,
    {
        fn provider() -> Provider {
            Provider::Class(ClassProvider {
                provide: ServiceIdentifier::from(ServiceType::create::<Self>()),
                common: ProviderCommon {
                    lifetime: ServiceLifetime::Singleton,
                    primary: false,
                    lazy: None,
                    source: ServiceSource::new("provider_definition.rs", 1, 1),
                    cleanup: None,
                },
                dependencies: vec![],
                constructor: construct_repository::<T>,
            })
        }
    }

    #[test]
    fn closed_provider_callback_is_specialized_for_the_closed_dependency_type() {
        let callback: ClosedProviderCallback = provider_definition::<Repository<Entity>>;
        let injection = DependencyRequest {
            declaration_position: 0,
            input_slot: InputSlot::new(0),
            label: Some("repository"),
            token: ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>()),
            optional: false,
            lazy: None,
            project: None,
            delivery: Delivery::Direct(prepare_required::<Repository<Entity>>),
            provider_source: ProviderSource::Materialize(callback),
        };
        let definition = match injection.provider_source {
            ProviderSource::Materialize(definition) => definition,
            ProviderSource::Registered => {
                panic!("closed generic injection should retain its provider callback")
            }
        };

        let Provider::Class(ClassProvider {
            provide,
            common,
            dependencies,
            constructor,
        }) = definition()
        else {
            panic!("closed generic callback should produce a class provider")
        };

        assert_eq!(
            provide,
            ServiceIdentifier::from(ServiceType::create::<Repository<Entity>>())
        );
        assert_eq!(common.lifetime, ServiceLifetime::Singleton);
        assert!(!common.primary);
        assert!(dependencies.is_empty());

        let erased = constructor(ConstructionInputs::empty())
            .expect("closed generic provider should construct its concrete type");
        assert!(erased.downcast::<Repository<Entity>>().is_ok());
    }
}

mod service_identifier {
    use super::{ServiceIdentifier, ServiceType};

    #[test]
    fn create_service_identifier_by_type() {
        struct Test;

        let service_type = ServiceType::create::<Test>();

        let service_identifier = ServiceIdentifier::from(service_type);

        assert_eq!(service_identifier.service_type, service_type);
        assert_eq!(service_identifier.service_key, None);
    }

    #[test]
    fn create_service_identifier_by_generic_type() {
        trait Repository<T>: Send + Sync + 'static {}

        struct User;
        struct Post;

        let service_type1 = ServiceType::create::<dyn Repository<User>>();
        let service_identifier1 = ServiceIdentifier::from(service_type1);

        let service_type2 = ServiceType::create::<dyn Repository<Post>>();
        let service_identifier2 = ServiceIdentifier::from(service_type2);

        assert_ne!(
            service_identifier1, service_identifier2,
            "service_identifier1 与 service_identifier2 服务相同"
        );
    }
}

mod escaped_adapter {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::{
        activation::Injection, graph::GraphCompiler, registration::provider::CleanupFuture,
        runtime::Runtime,
    };

    static DEPENDENCY_DROPS: AtomicUsize = AtomicUsize::new(0);
    static DEPENDENCY_CLEANUPS: AtomicUsize = AtomicUsize::new(0);
    static ESCAPED: Mutex<Option<Injection<Dependency>>> = Mutex::new(None);

    struct Dependency {
        value: u32,
    }

    impl Drop for Dependency {
        fn drop(&mut self) {
            DEPENDENCY_DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct Consumer;

    fn cleanup_dependency() -> CleanupFuture {
        Box::pin(async {
            DEPENDENCY_CLEANUPS.fetch_add(1, Ordering::SeqCst);
        })
    }

    fn construct_dependency(
        inputs: ConstructionInputs,
    ) -> Result<ErasedService, ConstructionError> {
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(Dependency { value: 91 }))
    }

    fn construct_consumer(
        mut inputs: ConstructionInputs,
    ) -> Result<ErasedService, ConstructionError> {
        let dependency = inputs.take::<Dependency>(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        *ESCAPED.lock().unwrap() = Some(dependency);
        Ok(ErasedService::new(Consumer))
    }

    fn common() -> ProviderCommon {
        ProviderCommon {
            lifetime: ServiceLifetime::Singleton,
            primary: false,
            lazy: None,
            source: ServiceSource::new(file!(), line!(), column!()),
            cleanup: None,
        }
    }

    #[tokio::test]
    async fn safe_adapter_escape_survives_disposal_and_drops_only_after_the_final_token() {
        let dependency = Provider::Class(ClassProvider {
            provide: ServiceIdentifier::from(ServiceType::create::<Dependency>()),
            common: ProviderCommon {
                cleanup: Some(cleanup_dependency),
                ..common()
            },
            dependencies: vec![],
            constructor: construct_dependency,
        });
        let consumer = Provider::Class(ClassProvider {
            provide: ServiceIdentifier::from(ServiceType::create::<Consumer>()),
            common: common(),
            dependencies: vec![DependencyRequest {
                declaration_position: 0,
                input_slot: InputSlot::new(0),
                token: ServiceIdentifier::from(ServiceType::create::<Dependency>()),
                optional: false,
                lazy: None,
                project: None,
                label: Some("captured_dependency"),
                delivery: Delivery::Direct(prepare_required::<Dependency>),
                provider_source: ProviderSource::Registered,
            }],
            constructor: construct_consumer,
        });
        let graph = Arc::new(
            GraphCompiler::compile_snapshot(crate::registration::catalog::RegistrySnapshot {
                providers: vec![dependency, consumer],
                bindings: vec![],
                roots: vec![],
                ..Default::default()
            })
            .unwrap(),
        );
        let consumer_id =
            graph.routes[&ServiceIdentifier::from(ServiceType::create::<Consumer>())].provider;
        let (runtime, owner) = Runtime::start(graph, 1);
        // 丢弃查询额外取得的 lease，让保活只依赖 journal 和逃逸的输入令牌，
        // 对应门面返回由 owner 支撑的引用时的真实所有权关系。
        drop(runtime.resolve(&owner, consumer_id).await.unwrap());
        assert_eq!(DEPENDENCY_DROPS.load(Ordering::SeqCst), 0);
        assert_eq!(DEPENDENCY_CLEANUPS.load(Ordering::SeqCst), 0);

        runtime.close(&owner).await.unwrap();
        assert_eq!(DEPENDENCY_CLEANUPS.load(Ordering::SeqCst), 1);
        assert_eq!(DEPENDENCY_DROPS.load(Ordering::SeqCst), 0);

        let escaped = ESCAPED.lock().unwrap().take().unwrap();
        assert_eq!(escaped.value, 91);
        drop(escaped);
        assert_eq!(DEPENDENCY_DROPS.load(Ordering::SeqCst), 1);
        assert_eq!(DEPENDENCY_CLEANUPS.load(Ordering::SeqCst), 1);
    }
}
