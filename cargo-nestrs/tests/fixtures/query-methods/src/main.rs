//! 普通查询方法、Self、泛型 helper、跨 crate 和未执行分支的集成契约。
use indirect_library as _;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use query_library::trait_calls::{self, CustomQuery, Query};
use query_library::{Repository, generic_query, nested_query};
use std::ops::Add;

struct User;
struct Order;
struct ThroughSelf;
struct ThroughClosure;
struct ThroughFunctionItem;
struct ThroughAssociated;
struct ThroughIterator;
struct ThroughIteratorHelper;
struct ThroughAddMethod;
struct ThroughAddOperator;
struct ThroughCustomTrait;
struct ThroughInherent;
struct ThroughDeadIterator;
struct ThroughDeadAddMethod;
struct ThroughDeadAddOperator;
struct Select;
impl query_library::SelectService for Select {
    type Service = Repository<ThroughAssociated>;
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(query_library::constructions(), 0);
    assert_eq!(trait_calls::constructions(), 0);
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    })
    .await
    .unwrap();
    // 闭合 helper、函数项、闭包、关联类型、const 参数以及服务方法的实例在 build 前进入计划。
    // 上游另含永不执行分支；只沿优化后可达代码收集将丢失这个实例。
    assert_eq!(
        query_library::constructions(),
        11 + usize::from(cfg!(feature = "extra-root"))
    );
    // 六个真实调用、三个本地未执行调用，以及一个上游私有未执行调用。
    // 每个调用使用不同 T，避免某个正常路径替遗漏路径意外补出相同查询根。
    assert_eq!(trait_calls::constructions(), 10);
    if false {
        drop(Query::<ThroughDeadIterator>::new(&provider).next());
        drop(Query::<ThroughDeadAddMethod>::new(&provider).add(()));
        drop(Query::<ThroughDeadAddOperator>::new(&provider) + ());
    }
    let mut trait_query_ids = [
        Query::<ThroughIterator>::new(&provider)
            .next()
            .unwrap()
            .await
            .unwrap(),
        trait_calls::iterator_query(Query::<ThroughIteratorHelper>::new(&provider))
            .await
            .unwrap(),
        Query::<ThroughAddMethod>::new(&provider)
            .add(())
            .await
            .unwrap(),
        (Query::<ThroughAddOperator>::new(&provider) + ())
            .await
            .unwrap(),
        Query::<ThroughCustomTrait>::new(&provider)
            .custom_query()
            .await
            .unwrap(),
        Query::<ThroughInherent>::new(&provider)
            .inherent_query()
            .await
            .unwrap(),
    ];
    trait_query_ids.sort_unstable();
    assert!(trait_query_ids.windows(2).all(|pair| pair[0] != pair[1]));
    assert_eq!(trait_calls::constructions(), 10);
    let workflow = provider
        .get_required_service::<dyn query_library::WorkflowPort>()
        .await
        .unwrap();
    let _ = workflow.query_through_dyn(&provider).await;
    let default_workflow = provider
        .get_required_service::<dyn query_library::DefaultWorkflowPort>()
        .await
        .unwrap();
    let _ = default_workflow.default_query(&provider).await;
    assert_eq!(
        provider
            .get_required_service::<query_library::SecretRepository<User>>()
            .await
            .unwrap()
            .secret(),
        707
    );
    let user = generic_query::<Repository<User>>(&provider).await.unwrap();
    let order = nested_query::<Order>(&provider).await.unwrap();
    let through_self = Repository::<ThroughSelf>::get_self(&provider)
        .await
        .unwrap();
    assert_ne!(user.id(), order.id());
    assert_ne!(user.id(), through_self.id());
    assert!(provider.get_service::<u64>().await.unwrap().is_none());
    let _ = query_library::closure_query::<ThroughClosure>(&provider)
        .await
        .unwrap();
    let query = generic_query::<Repository<ThroughFunctionItem>>;
    let _ = query(&provider).await.unwrap();
    let _ = query_library::const_query::<7>(&provider).await.unwrap();
    let _ = query_library::associated_query::<Select>(&provider)
        .await
        .unwrap();
    let scope = provider.create_scope();
    let same_user = scope
        .service_provider()
        .get_required_service::<Repository<User>>()
        .await
        .unwrap();
    assert!(std::ptr::eq(user, same_user));
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}
