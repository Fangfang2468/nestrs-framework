//! 编译计划装载协议测试。这里手写的是编译器最终写入序列，不是公开的动态注册 API。

use super::{
    ABSENT, PlanAssembly, load, plan_push_binding, plan_push_dependent, plan_push_order,
    plan_push_trait_route, plan_set_input, plan_set_options,
};
use crate::activation::adapter::{ActivationAdapter, Constructor, InputAdapter, ProjectionAdapter};
use crate::{
    InitializationMode, ServiceKey, ServiceLifetime,
    activation::{
        ActivationPreparation, ConstructionError, ConstructionInputs, ErasedService, InputSlot,
        prepare_bound_optional, prepare_bound_required, prepare_lazy_optional,
        prepare_optional_absent, prepare_required,
    },
    graph::{AbsentInput, DependencyInput},
    service::{ServiceIdentifier, ServiceSource, ServiceType},
};
use std::sync::Arc;

struct Dependency;
struct Consumer;
trait Port: Send + Sync {}
impl Port for Dependency {}
trait Missing: Send + Sync {}

fn source() -> ServiceSource {
    ServiceSource::new("compiled-plan-test.rs", 12, 3)
}

fn no_construction(_: ConstructionInputs) -> Result<ErasedService, ConstructionError> {
    panic!("装载编译计划不能执行用户构造")
}

fn identifier<T: ?Sized + Send + Sync + 'static>(key: Option<ServiceKey>) -> ServiceIdentifier {
    ServiceIdentifier::new(key, ServiceType::create::<T>())
}

fn adapter<T: Send + Sync + 'static>(inputs: Vec<InputAdapter>) -> ActivationAdapter {
    ActivationAdapter {
        service_type: ServiceType::create::<T>(),
        constructor: Constructor::Class(no_construction),
        inputs,
        cleanup: None,
    }
}

fn binding() -> ProjectionAdapter {
    ProjectionAdapter {
        trait_type: ServiceType::create::<dyn Port>(),
        concrete_type: ServiceType::create::<Dependency>(),
        prepare_required: |slot, value| {
            prepare_bound_required::<Dependency, dyn Port>(slot, value, |value| value)
        },
        prepare_optional: |slot, value| {
            prepare_bound_optional::<Dependency, dyn Port>(slot, value, |value| value)
        },
        project: |slot, value, output| {
            crate::activation::project_bound::<Dependency, dyn Port>(slot, value, output, |value| {
                value
            })
        },
    }
}

fn inputs() -> Vec<InputAdapter> {
    vec![
        InputAdapter {
            service_type: ServiceType::create::<Dependency>(),
            prepare: Some(prepare_required::<Dependency>),
            lazy: None,
            project: None,
        },
        InputAdapter {
            service_type: ServiceType::create::<dyn Port>(),
            prepare: None,
            lazy: None,
            project: None,
        },
        InputAdapter {
            service_type: ServiceType::create::<dyn Missing>(),
            prepare: Some(prepare_optional_absent::<dyn Missing>),
            lazy: Some(prepare_lazy_optional::<dyn Missing>),
            project: None,
        },
    ]
}

/// 固定 fixture 的 Transient 策略；只转交执行能力，不保存或解释服务声明。
unsafe fn push_transient(output: *mut (), adapter: ActivationAdapter, key: &'static str) {
    // SAFETY: 调用方提供同步使用的唯一 PlanAssembly 地址，类型能力来自当前测试。
    unsafe {
        super::plan_push_provider(
            output,
            adapter,
            2,
            usize::from(!key.is_empty()),
            key,
            0,
            0,
            source().file,
            source().line as usize,
            source().column as usize,
            false,
        );
    }
}

#[test]
fn compiled_indices_load_exact_routes_all_slots_and_typed_adapters_without_construction() {
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    // SAFETY: output 唯一指向本测试中的装配器，所有调用同步完成，没有保存或逃逸借用。
    unsafe {
        plan_set_options(output, true, 7);
        plan_push_binding(output, binding());
        push_transient(output, adapter::<Consumer>(inputs()), "");
        push_transient(output, adapter::<Dependency>(vec![]), "primary");
        // 不依赖写入顺序；每个槽位保存独立 occurrence 的选择，反向拓扑边才去重。
        plan_set_input(output, 0, 2, ABSENT, ABSENT, true, 0, "", 0, "missing");
        plan_set_input(output, 0, 1, 1, 0, false, 1, "primary", 0, "second");
        plan_set_input(output, 0, 0, 1, ABSENT, false, 1, "primary", 0, "first");
        plan_push_trait_route(output, 1, 0);
        plan_push_order(output, 1);
        plan_push_order(output, 0);
        plan_push_dependent(output, 1, 0);
    }
    let application = assembly.finish();
    assert_eq!(
        application.options.initialization,
        InitializationMode::Eager
    );
    assert_eq!(application.options.max_concurrent_activations.get(), 7);
    let graph = &application.graph;
    assert_eq!(graph.nodes.len(), 2);
    assert_eq!(graph.topological_order, [1, 0]);
    assert_eq!(graph.dependents, [vec![], vec![0]]);
    assert_eq!(graph.nodes[0].dependencies.len(), 3);
    assert_eq!(graph.nodes[0].dependencies[0].input.target(), Some(1));
    assert_eq!(graph.nodes[0].dependencies[1].input.target(), Some(1));
    assert_eq!(graph.nodes[0].dependencies[2].input.target(), None);
    assert!(graph.nodes[0].dependencies[2].input.is_lazy());
    assert!(graph.nodes[0].dependencies[2].input.lazy_plan().is_none());
    assert_eq!(
        graph.routes[&identifier::<dyn Port>(Some(ServiceKey::Named("primary".into())))].provider,
        1
    );
    assert!(!graph.routes.contains_key(&identifier::<dyn Port>(None)));
    assert!(!graph.routes.contains_key(&identifier::<dyn Missing>(None)));
    let DependencyInput::Absent(AbsentInput::Lazy(prepare)) = graph.nodes[0].dependencies[2].input
    else {
        panic!("缺席的延迟输入必须交付 Option<LazyInjection<T>>")
    };
    let mut preparation = ActivationPreparation::new(1);
    preparation
        .prepare_lazy(InputSlot::new(0), prepare, None)
        .unwrap();
    let (mut inputs, leases) = preparation.finish_class().unwrap();
    assert!(
        inputs
            .take_optional_lazy::<dyn Missing>(InputSlot::new(0))
            .unwrap()
            .is_none()
    );
    inputs.ensure_all_consumed().unwrap();
    assert!(leases.is_empty(), "缺席输入不能分配依赖 lease");
}

#[test]
fn absent_immediate_and_lazy_slots_deliver_distinct_optional_token_types() {
    let mut requests = vec![inputs().remove(2); 2];
    requests[0].lazy = None;
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    // SAFETY: 一个 provider 的两个连续槽位均由编译器决定缺席，没有外部地址或回调执行。
    unsafe {
        push_transient(output, adapter::<Consumer>(requests), "");
        plan_set_input(output, 0, 0, ABSENT, ABSENT, true, 0, "", 0, "immediate");
        plan_set_input(output, 0, 1, ABSENT, ABSENT, true, 0, "", 0, "lazy");
        plan_push_order(output, 0);
    }
    let application = assembly.finish();
    let dependencies = &application.graph.nodes[0].dependencies;
    let DependencyInput::Absent(AbsentInput::Immediate(immediate)) = dependencies[0].input else {
        panic!("普通 optional 缺席必须固定为立即输入")
    };
    let DependencyInput::Absent(AbsentInput::Lazy(lazy)) = dependencies[1].input else {
        panic!("延迟 optional 缺席必须保留延迟令牌类型")
    };
    // 两个 None 在业务上都表示缺席，在 ABI 上却是不同的 Rust 类型；同时走真实
    // 准备事务和 typed extraction，避免仅比较 enum 分支而漏掉错误准备函数。
    let mut preparation = ActivationPreparation::new(2);
    preparation
        .prepare(InputSlot::new(0), immediate, None)
        .unwrap();
    preparation
        .prepare_lazy(InputSlot::new(1), lazy, None)
        .unwrap();
    let (mut inputs, leases) = preparation.finish_class().unwrap();
    let immediate: Option<crate::Injection<dyn Missing>> =
        inputs.take_optional(InputSlot::new(0)).unwrap();
    let lazy: Option<crate::LazyInjection<dyn Missing>> =
        inputs.take_optional_lazy(InputSlot::new(1)).unwrap();
    assert!(immediate.is_none());
    assert!(lazy.is_none());
    inputs.ensure_all_consumed().unwrap();
    assert!(leases.is_empty());
}

#[test]
fn one_entry_shares_immutable_plan_between_loads() {
    let first = load();
    let second = load();
    assert!(std::ptr::eq(first, second));
    assert!(Arc::ptr_eq(&first.graph, &second.graph));
}

#[test]
fn provider_initialization_overrides_survive_plan_loading_without_global_folding() {
    for default_eager in [false, true] {
        let mut assembly = PlanAssembly::default();
        let output = (&mut assembly as *mut PlanAssembly).cast();
        // 描述回调只装载三态策略；即使当前入口默认值相同，也不能折叠掉继承状态，
        // 因为未来的 build_with_options 可以让另一个 root 使用不同默认值。
        // SAFETY: output 是当前唯一装配器，每个无输入 provider 有唯一 key 和有效编号。
        unsafe {
            plan_set_options(output, default_eager, 3);
            for (index, lazy) in [None, Some(true), Some(false)].into_iter().enumerate() {
                let initialization = match lazy {
                    None => 0,
                    Some(true) => 1,
                    Some(false) => 2,
                };
                super::plan_push_provider(
                    output,
                    adapter::<Dependency>(vec![]),
                    2,
                    2,
                    "",
                    index,
                    initialization,
                    source().file,
                    12,
                    3,
                    false,
                );
                plan_push_order(output, index);
            }
        }
        let plan = assembly.finish();
        assert_eq!(plan.graph.nodes.len(), 3);
        let modes: Vec<_> = plan
            .graph
            .nodes
            .iter()
            .map(|node| node.common.lazy)
            .collect();
        assert_eq!(modes, [None, Some(true), Some(false)]);
        let snapshot = super::super::snapshot(&plan.graph);
        let modes: Vec<_> = snapshot["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["initialization"].as_str().unwrap())
            .collect();
        assert_eq!(modes, ["inherit", "lazy", "eager"]);
    }
}

#[test]
fn lazy_edges_freeze_selected_projection_and_absence_once() {
    let mut requests = inputs();
    requests[0].lazy = Some(crate::activation::prepare_lazy_required::<Dependency>);
    requests[0].project = Some(crate::activation::project_required::<Dependency>);
    requests[1].lazy = Some(crate::activation::prepare_lazy_required::<dyn Port>);
    // trait 输入的直接投影必须来自已经选定的 binding，而不是消费点的 fallback。
    requests[1].project = Some(crate::activation::project_required::<Dependency>);
    let binding = binding();
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    // SAFETY: 当前装配器由本测试独占，节点、binding 和输入编号满足内部协议。
    unsafe {
        plan_push_binding(output, binding);
        push_transient(output, adapter::<Consumer>(requests), "checkout");
        push_transient(output, adapter::<Dependency>(vec![]), "primary");
        plan_set_input(output, 0, 0, 1, ABSENT, false, 1, "primary", 0, "first");
        plan_set_input(output, 0, 1, 1, 0, false, 1, "primary", 0, "second");
        plan_set_input(output, 0, 2, ABSENT, ABSENT, true, 0, "", 0, "missing");
        plan_push_order(output, 1);
        plan_push_order(output, 0);
        plan_push_dependent(output, 1, 0);
    }
    let application = assembly.finish();
    let inputs = &application.graph.nodes[0].dependencies;
    let direct = inputs[0].input.lazy_plan().unwrap();
    let bound = inputs[1].input.lazy_plan().unwrap();
    assert_eq!(direct.provider, 1);
    assert_eq!(direct.input, InputSlot::new(0));
    assert_eq!(direct.source, source());
    assert_eq!(direct.label, Some("first"));
    assert_eq!(
        direct.consumer,
        identifier::<Consumer>(Some(ServiceKey::Named("checkout".into())))
    );
    assert!(!Arc::ptr_eq(direct, bound), "重复目标仍有各自的输入描述");
    assert!(std::ptr::fn_addr_eq(bound.project, binding.project));
    assert!(
        inputs[2].input.lazy_plan().is_none(),
        "缺席字段不分配延迟计划"
    );
}

#[tokio::test]
async fn lazy_metadata_is_shared_across_occurrences_and_survives_owner_and_graph_drop() {
    struct LazyConsumer {
        dependency: crate::LazyInjection<Dependency>,
    }
    let request = InputAdapter {
        service_type: ServiceType::create::<Dependency>(),
        prepare: Some(prepare_required::<Dependency>),
        lazy: Some(crate::activation::prepare_lazy_required::<Dependency>),
        project: Some(crate::activation::project_required::<Dependency>),
    };
    let mut consumer = adapter::<LazyConsumer>(vec![request]);
    consumer.constructor = Constructor::Class(|mut inputs| {
        let dependency = inputs.take_lazy(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(LazyConsumer { dependency }))
    });
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    // SAFETY: 计划只含一条合法延迟边，装配同步且地址未逃逸。
    unsafe {
        push_transient(output, consumer, "lazy_owner");
        push_transient(output, adapter::<Dependency>(vec![]), "");
        plan_set_input(output, 0, 0, 1, ABSENT, false, 0, "", 0, "deferred");
        plan_push_order(output, 1);
        plan_push_order(output, 0);
        plan_push_dependent(output, 1, 0);
    }
    let application = assembly.finish();
    let plan = application.graph.nodes[0].dependencies[0]
        .input
        .lazy_plan()
        .unwrap()
        .clone();
    let weak_plan = Arc::downgrade(&plan);
    assert_eq!(Arc::strong_count(&plan), 2);
    let (first_runtime, first_root) = crate::runtime::Runtime::start(application.graph.clone(), 2);
    let (second_runtime, second_root) =
        crate::runtime::Runtime::start(application.graph.clone(), 2);
    let first_scope = first_runtime.create_scope();
    let second_scope = first_runtime.create_scope();
    let consumers = [
        first_runtime.resolve(&first_root, 0).await.unwrap(),
        first_runtime.resolve(&first_root, 0).await.unwrap(),
        first_runtime.resolve(&first_scope, 0).await.unwrap(),
        first_runtime.resolve(&second_scope, 0).await.unwrap(),
        second_runtime.resolve(&second_root, 0).await.unwrap(),
    ];
    // 每个真实字段仅增加共享描述的一份强引用，没有独立 key/源码/投影描述分配。
    assert_eq!(Arc::strong_count(&plan), 2 + consumers.len());
    assert!(
        !consumers[0].ptr_eq(&consumers[1]),
        "Transient occurrence 仍须独立"
    );
    first_runtime.close(&first_root).await.unwrap();
    second_runtime.close(&second_root).await.unwrap();
    drop((
        first_scope,
        second_scope,
        first_root,
        second_root,
        first_runtime,
        second_runtime,
    ));
    drop(application);
    drop(plan);
    assert!(weak_plan.upgrade().is_some());
    // SAFETY: 每个局部 lease 都仍保活准确的 LazyConsumer，owner 关闭不释放逃逸实例。
    let consumer = unsafe { consumers[0].pointer::<LazyConsumer>().unwrap().as_ref() };
    let error = consumer
        .dependency
        .get()
        .await
        .err()
        .expect("已关闭 owner 不能首次构造依赖");
    assert!(error.to_string().contains("lazy_owner"));
    assert!(error.to_string().contains("deferred"));
    drop(consumers);
    assert!(
        weak_plan.upgrade().is_none(),
        "字段释放后不能遗留描述的持有环"
    );
}

#[test]
#[should_panic(expected = "Nestrs 编译计划的延迟输入缺少直接类型化投影")]
fn incompatible_lazy_plan_rejects_a_missing_direct_projection() {
    let mut requests = inputs();
    requests.truncate(1);
    requests[0].lazy = Some(crate::activation::prepare_lazy_required::<Dependency>);
    requests[0].project = None;
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    // SAFETY: 指针与编号合法；刻意省略直接投影以验证 ABI 损坏不会退回旧装箱路径。
    unsafe {
        push_transient(output, adapter::<Consumer>(requests), "");
        push_transient(output, adapter::<Dependency>(vec![]), "primary");
        plan_set_input(output, 0, 0, 1, ABSENT, false, 1, "primary", 0, "first");
    }
}

#[tokio::test]
async fn sharing_compiled_plan_does_not_share_root_instances_failures_or_closing() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RootInstance(usize);
    struct FailsOncePerRoot;
    static CREATED: AtomicUsize = AtomicUsize::new(0);
    static FAILED: AtomicUsize = AtomicUsize::new(0);
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    let mut instance = adapter::<RootInstance>(vec![]);
    instance.constructor = Constructor::Class(|_| {
        Ok(ErasedService::new(RootInstance(
            CREATED.fetch_add(1, Ordering::SeqCst),
        )))
    });
    let mut failure = adapter::<FailsOncePerRoot>(vec![]);
    failure.constructor = Constructor::Class(|_| {
        FAILED.fetch_add(1, Ordering::SeqCst);
        Err(ConstructionError::RequiredDependencyAbsent {
            slot: InputSlot::new(0),
        })
    });
    // SAFETY: 当前装配器地址唯一、同步使用；两个没有输入的节点使用有效计划编号。
    unsafe {
        super::plan_push_provider(
            output,
            instance,
            0,
            0,
            "",
            0,
            0,
            source().file,
            12,
            3,
            false,
        );
        super::plan_push_provider(output, failure, 0, 0, "", 0, 0, source().file, 12, 3, false);
        plan_push_order(output, 0);
        plan_push_order(output, 1);
    }
    let application = assembly.finish();
    assert_eq!(CREATED.load(Ordering::SeqCst), 0);
    let (first_runtime, first_root) = crate::runtime::Runtime::start(application.graph.clone(), 2);
    let (second_runtime, second_root) =
        crate::runtime::Runtime::start(application.graph.clone(), 2);
    let first = first_runtime.resolve(&first_root, 0).await.unwrap();
    let first_again = first_runtime.resolve(&first_root, 0).await.unwrap();
    let second = second_runtime.resolve(&second_root, 0).await.unwrap();
    assert!(first.ptr_eq(&first_again));
    assert!(!first.ptr_eq(&second));
    assert_eq!(CREATED.load(Ordering::SeqCst), 2);
    // SAFETY: 每个 pointer 的准确类型已由 ErasedService 检查，局部 lease 仍持有实例。
    unsafe {
        assert_ne!(
            first.pointer::<RootInstance>().unwrap().as_ref().0,
            second.pointer::<RootInstance>().unwrap().as_ref().0
        );
    }
    for _ in 0..2 {
        assert!(first_runtime.resolve(&first_root, 1).await.is_err());
    }
    for _ in 0..2 {
        assert!(second_runtime.resolve(&second_root, 1).await.is_err());
    }
    assert_eq!(FAILED.load(Ordering::SeqCst), 2);
    first_runtime.close(&first_root).await.unwrap();
    assert!(first_runtime.resolve(&first_root, 0).await.is_err());
    assert!(
        second_runtime
            .resolve(&second_root, 0)
            .await
            .unwrap()
            .ptr_eq(&second)
    );
    second_runtime.close(&second_root).await.unwrap();
}

#[test]
#[should_panic(expected = "Nestrs 编译计划缺少输入选择")]
fn incompatible_plan_cannot_publish_unassigned_input() {
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    // SAFETY: 指针协议有效；刻意省略输入赋值，验证版本/生成器错误不会被静默接受。
    unsafe {
        push_transient(output, adapter::<Consumer>(inputs()), "");
        plan_push_order(output, 0);
    }
    assembly.finish();
}

#[test]
fn reflect_execution_contract_loads_keys_policy_and_sources_without_declarations() {
    // 直接模拟工具链最终入口。三个节点复用同一
    // 类型化构造能力，key 与执行策略全部来自最终计划的确定标量。
    let mut assembly = PlanAssembly::default();
    let output = (&mut assembly as *mut PlanAssembly).cast();
    for (index, (key_kind, key_name, key_index)) in
        [(0, "", 0), (1, "billing", 0), (2, "", usize::MAX)]
            .into_iter()
            .enumerate()
    {
        let adapter = ActivationAdapter {
            service_type: ServiceType::create::<Dependency>(),
            constructor: crate::activation::adapter::Constructor::Class(no_construction),
            inputs: vec![],
            cleanup: None,
        };
        // SAFETY: 指针唯一且同步使用，每个节点的 key 唯一，顺序编号完整。
        unsafe {
            super::plan_push_provider(
                output,
                adapter,
                index,
                key_kind,
                key_name,
                key_index,
                index,
                "reflect-generated.rs",
                42,
                7,
                index == 1,
            );
            super::plan_push_order(output, index);
        }
    }
    let application = assembly.finish();
    let graph = application.graph;
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.nodes[0].common.lifetime, ServiceLifetime::Singleton);
    assert_eq!(graph.nodes[1].common.lifetime, ServiceLifetime::Scoped);
    assert_eq!(graph.nodes[2].common.lifetime, ServiceLifetime::Transient);
    assert_eq!(graph.nodes[0].common.lazy, None);
    assert_eq!(graph.nodes[1].common.lazy, Some(true));
    assert_eq!(graph.nodes[2].common.lazy, Some(false));
    assert_eq!(
        graph.nodes[0].common.source,
        ServiceSource::new("reflect-generated.rs", 42, 7)
    );
    assert_eq!(graph.routes[&identifier::<Dependency>(None)].provider, 0);
    assert_eq!(
        graph.routes[&identifier::<Dependency>(Some(ServiceKey::Named("billing".into())))].provider,
        1
    );
    assert_eq!(
        graph.routes[&identifier::<Dependency>(Some(ServiceKey::Indexed(usize::MAX)))].provider,
        2
    );
}
