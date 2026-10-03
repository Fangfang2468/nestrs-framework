//! 真实类型身份与构造令牌的安全契约；声明发现和泛型物化由工具链契约测试覆盖。

use crate::{
    ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, InputSlot, prepare_required,
    },
    graph::{
        CompiledDependency, CompiledNode, Constructor, DependencyInput, NodePolicy, ValidatedGraph,
    },
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};

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
        activation::{Injection, adapter::CleanupFuture},
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

    fn common() -> NodePolicy {
        NodePolicy {
            lifetime: ServiceLifetime::Singleton,
            lazy: None,
            source: ServiceSource::new(file!(), line!(), column!()),
            cleanup: None,
        }
    }

    #[tokio::test]
    async fn safe_adapter_escape_survives_disposal_and_drops_only_after_the_final_token() {
        let dependency = CompiledNode {
            identifier: ServiceIdentifier::from(ServiceType::create::<Dependency>()),
            common: NodePolicy {
                cleanup: Some(cleanup_dependency),
                ..common()
            },
            dependencies: vec![],
            constructor: Constructor::Class(construct_dependency),
            requires_scope: false,
        };
        let consumer = CompiledNode {
            identifier: ServiceIdentifier::from(ServiceType::create::<Consumer>()),
            common: common(),
            dependencies: vec![CompiledDependency {
                slot: InputSlot::new(0),
                requested: dependency.identifier.clone(),
                optional: false,
                input: DependencyInput::Immediate {
                    target: 0,
                    prepare: prepare_required::<Dependency>,
                },
                label: Some("captured_dependency"),
            }],
            constructor: Constructor::Class(construct_consumer),
            requires_scope: false,
        };
        // 本测试只验证执行和所有权；候选与拓扑由 fixture 明确指定，不重复图编译。
        let consumer_id = 1;
        let graph = Arc::new(ValidatedGraph {
            nodes: vec![dependency, consumer],
            topological_order: vec![0, 1],
            dependents: vec![vec![1], vec![]],
            routes: Default::default(),
        });
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
