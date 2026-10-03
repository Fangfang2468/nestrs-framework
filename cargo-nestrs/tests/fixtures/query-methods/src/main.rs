//! 普通查询方法、Self、泛型 helper、跨 crate 和未执行分支的集成契约。
use indirect_library as _;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};
use query_library::associated_consts::{
    self as upstream_consts, DefaultQuery, ExplicitQuery, Holder, OverriddenQuery,
};
use query_library::forwarding_queries;
use query_library::trait_calls::{self, CustomQuery, Query};
use query_library::{Repository, generic_query, nested_query};
use std::ops::Add;

mod associated_consts;

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
struct ThroughUpstreamConst;
struct ThroughUpstreamConstChain;
struct ThroughUpstreamConstDefault;
struct ThroughUpstreamConstExplicit;
struct ThroughUpstreamGenericTraitConst;
struct ThroughUpstreamOverriddenConst;
struct ThroughUpstreamConstFn;
struct ThroughUpstreamInlineConst;
struct ThroughDeadUpstreamConst;
struct ThroughCollect;
struct ThroughGenericCollect;
struct ThroughGenericTypeCollect;
struct ThroughIteratorAdapters;
struct ThroughOverriddenCollect;
struct ThroughNestedCollect;
#[cfg(feature = "extra-root")]
struct ThroughFeatureUpstreamConst;
struct Select;
impl query_library::SelectService for Select {
    type Service = Repository<ThroughAssociated>;
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    assert_eq!(query_library::constructions(), 0);
    assert_eq!(trait_calls::constructions(), 0);
    assert_eq!(associated_consts::constructions(), 0);
    assert_eq!(upstream_consts::constructions(), 0);
    assert_eq!(forwarding_queries::constructions(), 0);
    let provider = ServiceProvider::build(Some(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        ..Default::default()
    }))
    .await
    .unwrap();
    // 六个上游 static（含私有未调用项和数组）和五条不同的标准库转发路径。
    // 先核对 Eager 计数，再执行回调，避免运行时路径替漏掉的冻结计划补根。
    assert_eq!(forwarding_queries::constructions(), 11);
    let overridden: Vec<_> =
        forwarding_queries::OverriddenQuery::<ThroughOverriddenCollect>::new(&provider).collect();
    assert!(overridden.is_empty());
    drop(overridden);
    let mut forwarded_ids = vec![
        forwarding_queries::DIRECT(&provider).await.unwrap(),
        forwarding_queries::ASSOCIATED(&provider).await.unwrap(),
        forwarding_queries::CONST_FN(&provider).await.unwrap(),
        forwarding_queries::INLINE(&provider).await.unwrap(),
        forwarding_queries::ARRAY[0](&provider).await.unwrap(),
    ];
    let forwarded = [
        forwarding_queries::Query::<ThroughCollect>::new(&provider).collect::<Vec<_>>(),
        forwarding_queries::generic_iterator(
            forwarding_queries::Query::<ThroughGenericCollect>::new(&provider),
        ),
        forwarding_queries::generic_type::<ThroughGenericTypeCollect>(&provider),
        forwarding_queries::OuterQuery::<ThroughNestedCollect>::new(&provider).collect::<Vec<_>>(),
        forwarding_queries::Query::<ThroughIteratorAdapters>::new(&provider)
            .map(|future| future)
            .take(1)
            .collect::<Vec<_>>(),
    ];
    for queries in forwarded {
        assert_eq!(queries.len(), 1);
        forwarded_ids.push(queries.into_iter().next().unwrap().await.unwrap());
    }
    forwarded_ids.sort_unstable();
    assert!(forwarded_ids.windows(2).all(|pair| pair[0] != pair[1]));
    assert_eq!(forwarding_queries::constructions(), 11);
    // 闭合 helper、函数项、闭包、关联类型、const 参数以及服务方法的实例在 build 前进入计划。
    // 上游另含永不执行分支；只沿优化后可达代码收集将丢失这个实例。
    assert_eq!(
        query_library::constructions(),
        11 + usize::from(cfg!(feature = "extra-root"))
    );
    // 六个真实调用、三个本地未执行调用，以及一个上游私有未执行调用。
    // 每个调用使用不同 T，避免某个正常路径替遗漏路径意外补出相同查询根。
    assert_eq!(trait_calls::constructions(), 10);
    // 关联常量使用独立计数器：本地八条实际路径加一个 if false；跨 crate
    // 八条实际路径、应用 if false 和上游私有 if false，feature 再各增加独立根。
    // 必须在第一次调用这些函数指针前检查，防止运行期初始化掩盖编译期漏根。
    assert_eq!(
        associated_consts::constructions(),
        associated_consts::expected_constructions()
    );
    let expected_upstream_consts = 10 + 2 * usize::from(cfg!(feature = "extra-root"));
    assert_eq!(upstream_consts::constructions(), expected_upstream_consts);
    associated_consts::verify(&provider).await;
    let query = Holder::<ThroughUpstreamConst>::QUERY;
    let mut const_query_ids = vec![
        query(&provider).await.unwrap(),
        Holder::<ThroughUpstreamConstChain>::CHAIN(&provider)
            .await
            .unwrap(),
        <Holder<ThroughUpstreamConstDefault> as DefaultQuery<ThroughUpstreamConstDefault>>::QUERY(
            &provider,
        )
        .await
        .unwrap(),
        <Holder<ThroughUpstreamConstExplicit> as ExplicitQuery<ThroughUpstreamConstExplicit>>::QUERY(
            &provider,
        )
        .await
        .unwrap(),
        upstream_consts::generic_trait_query::<
            Holder<ThroughUpstreamGenericTraitConst>,
            ThroughUpstreamGenericTraitConst,
        >(&provider)
        .await
        .unwrap(),
        <Holder<ThroughUpstreamOverriddenConst> as OverriddenQuery<
            ThroughUpstreamOverriddenConst,
        >>::QUERY(&provider)
        .await
        .unwrap(),
        Holder::<ThroughUpstreamConstFn>::THROUGH_CONST_FN(&provider)
            .await
            .unwrap(),
        upstream_consts::inline_query::<ThroughUpstreamInlineConst>(&provider)
            .await
            .unwrap(),
    ];
    if false {
        drop(Holder::<ThroughDeadUpstreamConst>::QUERY(&provider));
    }
    #[cfg(feature = "extra-root")]
    const_query_ids.push(
        Holder::<ThroughFeatureUpstreamConst>::QUERY(&provider)
            .await
            .unwrap(),
    );
    const_query_ids.sort_unstable();
    assert!(const_query_ids.windows(2).all(|pair| pair[0] != pair[1]));
    assert_eq!(upstream_consts::constructions(), expected_upstream_consts);
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
    let scope = provider.create_scope(None).await.unwrap();
    let same_user = scope
        .service_provider()
        .get_required_service::<Repository<User>>()
        .await
        .unwrap();
    assert!(std::ptr::eq(user, same_user));
    scope.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
}
