//! 编译计划装载协议测试。这里手写的是编译器最终写入序列，不是公开的动态注册 API。

use super::{ABSENT, CompiledApplication, PlanAssembly, load};
use crate::activation::adapter::{ActivationAdapter, InputAdapter, ProjectionAdapter};
use crate::{
    InitializationMode, ServiceKey, ServiceLifetime,
    activation::{
        ConstructionError, ConstructionInputs, ErasedService, InputSlot, prepare_bound_optional,
        prepare_bound_required, prepare_lazy_optional, prepare_optional_absent, prepare_required,
    },
    registration::{
        binding::TraitBinding,
        catalog::RegistrySnapshot,
        dependency::{Delivery, DependencyRequest, ProviderSource},
        provider::{ClassProvider, Provider, ProviderCommon},
    },
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

fn no_materialization() -> Provider {
    panic!("装载编译计划不能选择或物化 Provider 蓝图")
}

fn identifier<T: ?Sized + Send + Sync + 'static>(key: Option<ServiceKey>) -> ServiceIdentifier {
    ServiceIdentifier::new(key, ServiceType::create::<T>())
}

fn provider<T: Send + Sync + 'static>(
    key: Option<ServiceKey>,
    dependencies: Vec<DependencyRequest>,
) -> Provider {
    Provider::Class(ClassProvider {
        provide: identifier::<T>(key),
        common: ProviderCommon {
            lifetime: ServiceLifetime::Transient,
            primary: false,
            lazy: None,
            source: source(),
            cleanup: None,
        },
        dependencies,
        constructor: no_construction,
    })
}

fn binding() -> TraitBinding {
    TraitBinding {
        trait_type: ServiceType::create::<dyn Port>(),
        concrete_type: ServiceType::create::<Dependency>(),
        materialize: Some(no_materialization),
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
        source: source(),
    }
}

fn inputs() -> Vec<DependencyRequest> {
    vec![
        DependencyRequest {
            declaration_position: 0,
            input_slot: InputSlot::new(0),
            token: identifier::<Dependency>(Some(ServiceKey::Named("primary".into()))),
            optional: false,
            lazy: None,
            project: None,
            label: Some("first"),
            delivery: Delivery::Direct(prepare_required::<Dependency>),
            provider_source: ProviderSource::Materialize(no_materialization),
        },
        DependencyRequest {
            declaration_position: 1,
            input_slot: InputSlot::new(1),
            token: identifier::<dyn Port>(Some(ServiceKey::Named("primary".into()))),
            optional: false,
            lazy: None,
            project: None,
            label: Some("second"),
            delivery: Delivery::RequiresBinding,
            provider_source: ProviderSource::Registered,
        },
        DependencyRequest {
            declaration_position: 2,
            input_slot: InputSlot::new(2),
            token: identifier::<dyn Missing>(None),
            optional: true,
            lazy: Some(prepare_lazy_optional::<dyn Missing>),
            project: None,
            label: Some("missing"),
            delivery: Delivery::RequiresBindingOrAbsent(prepare_optional_absent::<dyn Missing>),
            provider_source: ProviderSource::Registered,
        },
    ]
}

#[test]
fn compiled_indices_load_exact_routes_all_slots_and_typed_adapters_without_callbacks() {
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    // SAFETY: output 唯一指向本测试中的装配器，所有调用同步完成，没有保存或逃逸借用。
    unsafe {
        plan_set_options(output, true, 7);
        plan_push_binding(output, binding());
        plan_push_provider(output, provider::<Consumer>(None, inputs()), false);
        plan_push_provider(
            output,
            provider::<Dependency>(Some(ServiceKey::Named("primary".into())), vec![]),
            false,
        );
        // 不依赖写入顺序；每个槽位保存独立 occurrence 的选择，反向拓扑边才去重。
        plan_set_input(output, 0, 2, ABSENT, ABSENT);
        plan_set_input(output, 0, 1, 1, 0);
        plan_set_input(output, 0, 0, 1, ABSENT);
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
    assert_eq!(graph.nodes[0].dependencies[0].target, Some(1));
    assert_eq!(graph.nodes[0].dependencies[1].target, Some(1));
    assert_eq!(graph.nodes[0].dependencies[2].target, None);
    assert!(graph.nodes[0].dependencies[2].lazy.is_some());
    assert!(graph.nodes[0].dependencies[2].lazy_plan.is_none());
    assert_eq!(
        graph.routes[&identifier::<dyn Port>(Some(ServiceKey::Named("primary".into())))].provider,
        1
    );
    assert!(!graph.routes.contains_key(&identifier::<dyn Port>(None)));
    assert!(!graph.routes.contains_key(&identifier::<dyn Missing>(None)));
    let absent = (graph.nodes[0].dependencies[2].prepare)(InputSlot::new(2), None).unwrap();
    assert!(
        absent
            .into_optional::<dyn Missing>(InputSlot::new(2))
            .unwrap()
            .is_none()
    );
}

#[test]
fn compiled_plan_matches_graph_oracle_without_repeating_graph_compilation() {
    // 参考图只在隔离测试里运行；生产 loader 消费的编号来自编译器。
    let mut dependencies = inputs();
    dependencies[0].provider_source = ProviderSource::Registered;
    let mut binding = binding();
    binding.materialize = None;
    let providers = vec![
        provider::<Consumer>(None, dependencies),
        provider::<Dependency>(Some(ServiceKey::Named("primary".into())), vec![]),
    ];
    let expected = super::super::GraphCompiler::compile_snapshot(RegistrySnapshot {
        providers: providers.clone(),
        bindings: vec![binding],
        ..Default::default()
    })
    .unwrap();
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    // SAFETY: 同上，测试按编译器完整协议提供有效编号和真实 descriptor。
    unsafe {
        plan_push_binding(output, binding);
        for provider in providers {
            plan_push_provider(output, provider, false);
        }
        plan_set_input(output, 0, 0, 1, ABSENT);
        plan_set_input(output, 0, 1, 1, 0);
        plan_set_input(output, 0, 2, ABSENT, ABSENT);
        plan_push_trait_route(output, 1, 0);
        plan_push_order(output, 1);
        plan_push_order(output, 0);
        plan_push_dependent(output, 1, 0);
    }
    let actual = assembly.finish();
    assert_eq!(
        super::super::snapshot(&actual.graph),
        super::super::snapshot(&expected)
    );
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
        let mut assembly = TestAssembly::default();
        let output = (&mut assembly as *mut TestAssembly).cast();
        // 描述回调只装载三态策略；即使当前入口默认值相同，也不能折叠掉继承状态，
        // 因为未来的 build_with_options 可以让另一个 root 使用不同默认值。
        // SAFETY: output 是当前唯一装配器，每个无输入 provider 有唯一 key 和有效编号。
        unsafe {
            plan_set_options(output, default_eager, 3);
            for (index, lazy) in [None, Some(true), Some(false)].into_iter().enumerate() {
                let mut declaration =
                    provider::<Dependency>(Some(ServiceKey::Indexed(index)), vec![]);
                let Provider::Class(class) = &mut declaration else {
                    unreachable!()
                };
                class.common.lazy = lazy;
                plan_push_provider(output, declaration, false);
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
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    // SAFETY: 当前装配器由本测试独占，节点、binding 和输入编号满足内部协议。
    unsafe {
        plan_push_binding(output, binding);
        plan_push_provider(
            output,
            provider::<Consumer>(Some(ServiceKey::Named("checkout".into())), requests),
            false,
        );
        plan_push_provider(
            output,
            provider::<Dependency>(Some(ServiceKey::Named("primary".into())), vec![]),
            false,
        );
        plan_set_input(output, 0, 0, 1, ABSENT);
        plan_set_input(output, 0, 1, 1, 0);
        plan_set_input(output, 0, 2, ABSENT, ABSENT);
        plan_push_order(output, 1);
        plan_push_order(output, 0);
        plan_push_dependent(output, 1, 0);
    }
    let application = assembly.finish();
    let inputs = &application.graph.nodes[0].dependencies;
    let direct = inputs[0].lazy_plan.as_ref().unwrap();
    let bound = inputs[1].lazy_plan.as_ref().unwrap();
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
    assert!(inputs[2].lazy_plan.is_none(), "缺席字段不分配延迟计划");
}

#[tokio::test]
async fn lazy_metadata_is_shared_across_occurrences_and_survives_owner_and_graph_drop() {
    struct LazyConsumer {
        dependency: crate::LazyInjection<Dependency>,
    }
    let request = DependencyRequest {
        declaration_position: 0,
        input_slot: InputSlot::new(0),
        token: identifier::<Dependency>(None),
        optional: false,
        lazy: Some(crate::activation::prepare_lazy_required::<Dependency>),
        project: Some(crate::activation::project_required::<Dependency>),
        label: Some("deferred"),
        delivery: Delivery::Direct(prepare_required::<Dependency>),
        provider_source: ProviderSource::Registered,
    };
    let mut consumer =
        provider::<LazyConsumer>(Some(ServiceKey::Named("lazy_owner".into())), vec![request]);
    let Provider::Class(definition) = &mut consumer else {
        unreachable!()
    };
    definition.constructor = |mut inputs| {
        let dependency = inputs.take_lazy(InputSlot::new(0))?;
        inputs.ensure_all_consumed()?;
        Ok(ErasedService::new(LazyConsumer { dependency }))
    };
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    // SAFETY: 计划只含一条合法延迟边，装配同步且地址未逃逸。
    unsafe {
        plan_push_provider(output, consumer, false);
        plan_push_provider(output, provider::<Dependency>(None, vec![]), false);
        plan_set_input(output, 0, 0, 1, ABSENT);
        plan_push_order(output, 1);
        plan_push_order(output, 0);
        plan_push_dependent(output, 1, 0);
    }
    let application = assembly.finish();
    let plan = application.graph.nodes[0].dependencies[0]
        .lazy_plan
        .as_ref()
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
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    // SAFETY: 指针与编号合法；刻意省略直接投影以验证 ABI 损坏不会退回旧装箱路径。
    unsafe {
        plan_push_provider(output, provider::<Consumer>(None, requests), false);
        plan_push_provider(
            output,
            provider::<Dependency>(Some(ServiceKey::Named("primary".into())), vec![]),
            false,
        );
        plan_set_input(output, 0, 0, 1, ABSENT);
    }
}

#[tokio::test]
async fn sharing_compiled_plan_does_not_share_root_instances_failures_or_closing() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RootInstance(usize);
    struct FailsOncePerRoot;
    static CREATED: AtomicUsize = AtomicUsize::new(0);
    static FAILED: AtomicUsize = AtomicUsize::new(0);
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    let mut instance = provider::<RootInstance>(None, vec![]);
    let Provider::Class(definition) = &mut instance else {
        unreachable!()
    };
    definition.common.lifetime = ServiceLifetime::Singleton;
    definition.constructor = |_| {
        Ok(ErasedService::new(RootInstance(
            CREATED.fetch_add(1, Ordering::SeqCst),
        )))
    };
    let mut failure = provider::<FailsOncePerRoot>(None, vec![]);
    let Provider::Class(definition) = &mut failure else {
        unreachable!()
    };
    definition.common.lifetime = ServiceLifetime::Singleton;
    definition.constructor = |_| {
        FAILED.fetch_add(1, Ordering::SeqCst);
        Err(ConstructionError::RequiredDependencyAbsent {
            slot: InputSlot::new(0),
        })
    };
    // SAFETY: 当前装配器地址唯一、同步使用；两个没有输入的节点使用有效计划编号。
    unsafe {
        plan_push_provider(output, instance, false);
        plan_push_provider(output, failure, false);
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
    let mut assembly = TestAssembly::default();
    let output = (&mut assembly as *mut TestAssembly).cast();
    // SAFETY: 指针协议有效；刻意省略输入赋值，验证版本/生成器错误不会被静默接受。
    unsafe {
        plan_push_provider(output, provider::<Consumer>(None, inputs()), false);
        plan_push_order(output, 0);
    }
    assembly.finish();
}

/// 旧参考输入与最终协议的测试桥接。只在测试中保存声明副本；生产装配器没有这张表。
/// 每次写入都转换为新协议的执行能力和标量，确保原有选择/生命周期测试继续覆盖装配。
#[derive(Default)]
struct TestAssembly {
    plan: PlanAssembly,
    inputs: Vec<Vec<DependencyRequest>>,
}

impl TestAssembly {
    fn finish(self) -> CompiledApplication {
        self.plan.finish()
    }
}

unsafe fn test_assembly<'a>(output: *mut ()) -> &'a mut TestAssembly {
    // SAFETY: 测试调用点均传入各自栈上 TestAssembly 的唯一地址，同步使用且不逃逸。
    unsafe { &mut *output.cast::<TestAssembly>() }
}

fn key_parts(key: Option<&ServiceKey>) -> (usize, &'static str, usize) {
    match key {
        None => (0, "", 0),
        // fixture key 只在装配调用中需要静态字符串。沿用明确的有限测试值，不泄漏堆字符串。
        Some(ServiceKey::Named(name)) => (
            1,
            match name.as_str() {
                "primary" => "primary",
                "checkout" => "checkout",
                "lazy_owner" => "lazy_owner",
                _ => panic!("unknown fixture key"),
            },
            0,
        ),
        Some(ServiceKey::Indexed(index)) => (2, "", *index),
    }
}

unsafe fn plan_push_provider(output: *mut (), provider: Provider, requires_scope: bool) {
    let (identifier, common, inputs, constructor) = match provider {
        Provider::Class(provider) => (
            provider.provide,
            provider.common,
            provider.dependencies,
            crate::activation::adapter::Constructor::Class(provider.constructor),
        ),
        Provider::Factory(provider) => (
            provider.provide,
            provider.common,
            provider.dependencies,
            crate::activation::adapter::Constructor::Factory(provider.invoker),
        ),
    };
    let adapter = ActivationAdapter {
        service_type: identifier.service_type,
        constructor,
        inputs: inputs
            .iter()
            .map(|request| InputAdapter {
                service_type: request.token.service_type,
                prepare: match request.delivery {
                    Delivery::Direct(prepare)
                    | Delivery::Selected(prepare)
                    | Delivery::RequiresBindingOrAbsent(prepare) => Some(prepare),
                    Delivery::RequiresBinding => None,
                },
                lazy: request.lazy,
                project: request.project,
            })
            .collect(),
        cleanup: common.cleanup,
    };
    // SAFETY: fixture 在本次同步写入期间独占装配器。
    let assembly = unsafe { test_assembly(output) };
    assembly.inputs.push(inputs);
    let (key_kind, key_name, key_index) = key_parts(identifier.service_key.as_ref());
    let lifetime = match common.lifetime {
        ServiceLifetime::Singleton => 0,
        ServiceLifetime::Scoped => 1,
        ServiceLifetime::Transient => 2,
    };
    let initialization = match common.lazy {
        None => 0,
        Some(true) => 1,
        Some(false) => 2,
    };
    // SAFETY: 所有执行入口/标量来自当前 fixture，原始声明和物化回调不交给生产装配器。
    unsafe {
        super::plan_push_provider(
            (&mut assembly.plan as *mut PlanAssembly).cast(),
            adapter,
            lifetime,
            key_kind,
            key_name,
            key_index,
            initialization,
            common.source.file,
            common.source.line as usize,
            common.source.column as usize,
            requires_scope,
        )
    };
}

unsafe fn plan_set_input(
    output: *mut (),
    provider: usize,
    slot: usize,
    target: usize,
    projection: usize,
) {
    // SAFETY: 测试的唯一同步装配器。
    let assembly = unsafe { test_assembly(output) };
    let request = &assembly.inputs[provider][slot];
    let (key_kind, key_name, key_index) = key_parts(request.token.service_key.as_ref());
    // SAFETY: 计划中的目标和投影编号由各个测试明确提供；这里仅携带原输入的诊断标量。
    unsafe {
        super::plan_set_input(
            (&mut assembly.plan as *mut PlanAssembly).cast(),
            provider,
            slot,
            target,
            projection,
            request.optional,
            key_kind,
            key_name,
            key_index,
            request.label.unwrap_or(""),
        )
    };
}

unsafe fn plan_push_binding(output: *mut (), binding: TraitBinding) {
    // SAFETY: 同一测试装配器的短期唯一访问。
    let assembly = unsafe { test_assembly(output) };
    let projection = ProjectionAdapter {
        trait_type: binding.trait_type,
        concrete_type: binding.concrete_type,
        prepare_required: binding.prepare_required,
        prepare_optional: binding.prepare_optional,
        project: binding.project,
    };
    // SAFETY: binding 的物化规则与源码声明不进入执行协议，只保留真实转换能力。
    unsafe {
        super::plan_push_binding((&mut assembly.plan as *mut PlanAssembly).cast(), projection)
    };
}

unsafe fn plan_set_options(output: *mut (), eager: bool, concurrency: usize) {
    // SAFETY: 同步传入当前测试装配器的唯一地址。
    let assembly = unsafe { test_assembly(output) };
    unsafe {
        super::plan_set_options(
            (&mut assembly.plan as *mut PlanAssembly).cast(),
            eager,
            concurrency,
        )
    };
}

unsafe fn plan_push_order(output: *mut (), provider: usize) {
    // SAFETY: 同步传入当前测试装配器的唯一地址。
    let assembly = unsafe { test_assembly(output) };
    unsafe { super::plan_push_order((&mut assembly.plan as *mut PlanAssembly).cast(), provider) };
}

unsafe fn plan_push_dependent(output: *mut (), dependency: usize, consumer: usize) {
    // SAFETY: 同步传入当前测试装配器的唯一地址。
    let assembly = unsafe { test_assembly(output) };
    unsafe {
        super::plan_push_dependent(
            (&mut assembly.plan as *mut PlanAssembly).cast(),
            dependency,
            consumer,
        )
    };
}

unsafe fn plan_push_trait_route(output: *mut (), provider: usize, projection: usize) {
    // SAFETY: 同步传入当前测试装配器的唯一地址。
    let assembly = unsafe { test_assembly(output) };
    unsafe {
        super::plan_push_trait_route(
            (&mut assembly.plan as *mut PlanAssembly).cast(),
            provider,
            projection,
        )
    };
}

#[test]
fn reflect_execution_contract_loads_keys_policy_and_sources_without_declarations() {
    // 直接模拟工具链最终入口，不通过上面的旧声明 oracle 桥接。三个节点复用同一
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
