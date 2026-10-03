use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{LazyDependency, LazyInjection};
use crate::{
    ResolveError,
    activation::{
        ActivationPreparation, ConstructionError, DependencyLease, ErasedService, ErasedServiceRef,
        Injection, InputSlot, LazyInputPlan, ProjectionTarget, ReleaseDomain, ServiceProjector,
        deferred::{LazyReceiver, LazyResolver},
        prepare_lazy_optional, prepare_lazy_required, project_bound, project_required,
    },
    service::{ServiceIdentifier, ServiceKey, ServiceSource, ServiceType},
};

struct Reports(u32);

trait ReportPort: Send + Sync {
    fn count(&self) -> u32;
}

impl ReportPort for Reports {
    fn count(&self) -> u32 {
        self.0
    }
}

struct Resolver {
    calls: AtomicUsize,
    result: Result<DependencyLease, ResolveError>,
}

impl LazyResolver for Resolver {
    fn request(&self, provider: usize) -> Result<LazyReceiver, &'static str> {
        assert_eq!(provider, 17, "槽位必须提交冻结计划中选定的 provider");
        self.calls.fetch_add(1, Ordering::SeqCst);
        // 请求能力只返回交付通道；typed 结果与持续接收端由字段的控制块持有。
        let (_, receiver) = tokio::sync::watch::channel(Some(self.result.clone()));
        Ok(receiver)
    }
}

tokio::task_local! {
    // 访问限制属于当前任务，与 resolver 的存活期独立；不同测试不共享可变开关。
    static WAIT_REJECTED: ();
}

fn check_wait_allowed() -> Result<(), &'static str> {
    if WAIT_REJECTED.try_with(|()| ()).is_ok() {
        Err("当前调用者不允许等待")
    } else {
        Ok(())
    }
}

fn resolver() -> Arc<Resolver> {
    Arc::new(Resolver {
        calls: AtomicUsize::new(0),
        result: Ok(DependencyLease::new(
            ErasedService::new(Reports(42)),
            vec![],
            ReleaseDomain::new(),
        )),
    })
}

fn plan(project: ServiceProjector, input: InputSlot) -> Arc<LazyInputPlan> {
    Arc::new(LazyInputPlan {
        provider: 17,
        consumer: ServiceIdentifier::new(None, ServiceType::create::<Reports>()),
        source: ServiceSource::new(file!(), line!(), 1),
        label: Some("reports"),
        input,
        project,
    })
}

fn with_plan<R: LazyResolver + 'static>(
    resolver: &Arc<R>,
    plan: Arc<LazyInputPlan>,
) -> LazyDependency {
    let erased: Arc<dyn LazyResolver> = resolver.clone();
    LazyDependency {
        plan,
        resolver: Arc::downgrade(&erased),
        check_wait_allowed,
    }
}

fn dependency<R: LazyResolver + 'static>(
    resolver: &Arc<R>,
    project: ServiceProjector,
    input: InputSlot,
) -> LazyDependency {
    with_plan(resolver, plan(project, input))
}

#[test]
fn lazy_injection_preserves_send_sync_coinduction_for_cyclic_service_types() {
    struct A {
        _b: LazyInjection<B>,
    }
    struct B {
        _a: Injection<A>,
    }
    fn send_sync<T: Send + Sync>() {}
    send_sync::<A>();
    send_sync::<B>();
    send_sync::<LazyInjection<dyn ReportPort>>();
}

#[tokio::test]
async fn factory_lazy_arguments_own_their_slots_after_the_borrow_frame_is_dropped() {
    let resolver = resolver();
    let mut preparation = ActivationPreparation::new(3);
    let required = InputSlot::new(0);
    let optional = InputSlot::new(1);
    let absent = InputSlot::new(2);
    preparation
        .prepare_lazy(
            required,
            prepare_lazy_required::<Reports>,
            Some(dependency(&resolver, project_required::<Reports>, required)),
        )
        .unwrap();
    preparation
        .prepare_lazy(
            optional,
            prepare_lazy_optional::<Reports>,
            Some(dependency(&resolver, project_required::<Reports>, optional)),
        )
        .unwrap();
    preparation
        .prepare_lazy(absent, prepare_lazy_optional::<Reports>, None)
        .unwrap();
    let mut frame = preparation.finish_factory().unwrap();
    let (first, second) = {
        let mut inputs = frame.inputs();
        // 错误的读取形态或类型不能消费参数，正确适配器仍可继续取出原句柄。
        assert!(matches!(
            inputs.take_optional_lazy::<Reports>(required),
            Err(ConstructionError::LazyOptionalInputExpected { .. })
        ));
        assert!(matches!(
            inputs.take_lazy::<String>(required),
            Err(ConstructionError::InputTypeMismatch { .. })
        ));
        let first = inputs.take_lazy::<Reports>(required).unwrap();
        assert!(matches!(
            inputs.take_lazy::<Reports>(required),
            Err(ConstructionError::SlotAlreadyConsumed { .. })
        ));
        let second = inputs
            .take_optional_lazy::<Reports>(optional)
            .unwrap()
            .unwrap();
        assert!(
            inputs
                .take_optional_lazy::<Reports>(absent)
                .unwrap()
                .is_none()
        );
        inputs.ensure_all_consumed().unwrap();
        (first, second)
    };
    // 没有目标实例就不能制造 frame lease；句柄不借用 frame，移出后允许立即释放帧。
    assert!(frame.into_dependencies().is_empty());
    tokio::task::yield_now().await;
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(first.get().await.unwrap().0, 42);
    assert_eq!(second.get().await.unwrap().0, 42);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
    drop(resolver);
    assert_eq!(first.get().await.unwrap().0, 42);
    assert_eq!(second.get().await.unwrap().0, 42);
}

#[test]
fn lazy_handles_and_optional_handles_are_one_machine_word() {
    let word = std::mem::size_of::<usize>();
    // dyn 的宽指针存放在控制块内，字段本身只拥有一个非空的控制块地址。
    assert_eq!(std::mem::size_of::<LazyInjection<Reports>>(), word);
    assert_eq!(std::mem::size_of::<LazyInjection<dyn ReportPort>>(), word);
    assert_eq!(std::mem::size_of::<Option<LazyInjection<Reports>>>(), word);
    assert_eq!(
        std::mem::size_of::<Option<LazyInjection<dyn ReportPort>>>(),
        word,
    );
}

#[tokio::test]
async fn moving_a_resolved_handle_preserves_its_target_and_releases_it_once() {
    struct TrackedReports(Arc<AtomicUsize>);
    impl Drop for TrackedReports {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(Resolver {
        calls: AtomicUsize::new(0),
        result: Ok(DependencyLease::new(
            ErasedService::new(TrackedReports(drops.clone())),
            vec![],
            ReleaseDomain::new(),
        )),
    });
    let token = LazyInjection::<TrackedReports>::new(
        dependency(
            &resolver,
            project_required::<TrackedReports>,
            InputSlot::new(0),
        ),
        InputSlot::new(0),
    );
    assert_eq!(Arc::strong_count(&resolver), 1, "字段只弱持有请求能力");
    let address = std::ptr::from_ref(token.get().await.unwrap());
    drop(resolver);
    assert_eq!(drops.load(Ordering::SeqCst), 0);

    // 移动字段会移动 Box 句柄，但不能移动控制块内缓存，也不能移动最终服务。
    let mut moved = Some(token);
    let token = moved.take().unwrap();
    assert_eq!(address, std::ptr::from_ref(token.get().await.unwrap()));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(token);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_cancelled_wait_resumes_its_accepted_result_after_resolver_release() {
    use std::{future::poll_fn, sync::Mutex, task::Poll};

    struct PendingResolver {
        calls: AtomicUsize,
        receiver: Mutex<Option<LazyReceiver>>,
    }
    impl LazyResolver for PendingResolver {
        fn request(&self, provider: usize) -> Result<LazyReceiver, &'static str> {
            assert_eq!(provider, 17);
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.receiver
                .lock()
                .unwrap()
                .take()
                .ok_or("同一字段不能重新提交请求")
        }
    }
    let (sender, receiver) = tokio::sync::watch::channel(None);
    let resolver = Arc::new(PendingResolver {
        calls: AtomicUsize::new(0),
        receiver: Mutex::new(Some(receiver)),
    });
    let token = LazyInjection::<Reports>::new(
        dependency(&resolver, project_required::<Reports>, InputSlot::new(0)),
        InputSlot::new(0),
    );
    assert_eq!(Arc::strong_count(&resolver), 1);
    let mut first = Box::pin(token.get());
    poll_fn(|cx| {
        assert!(first.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    let mut second = Box::pin(token.get());
    poll_fn(|cx| {
        assert!(second.as_mut().poll(cx).is_pending());
        Poll::Ready(())
    })
    .await;
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    drop(first);
    drop(resolver);

    // owner 请求能力消失不能取消已接受的交付。结果通道归槽位所有，与首次等待
    // future 和 owner 的存活期均无关；再次访问只接续，不升级已失效的 Weak。
    sender
        .send(Some(Ok(DependencyLease::new(
            ErasedService::new(Reports(42)),
            vec![],
            ReleaseDomain::new(),
        ))))
        .unwrap();
    drop(sender);
    assert_eq!(second.await.unwrap().0, 42);
    assert_eq!(token.get().await.unwrap().0, 42);
}

#[tokio::test]
async fn an_unrequested_handle_does_not_keep_its_resolver_alive() {
    let resolver = resolver();
    let weak = Arc::downgrade(&resolver);
    let plan = Arc::new(LazyInputPlan {
        provider: 17,
        consumer: ServiceIdentifier::new(
            Some(ServiceKey::Named("monthly-reports".to_owned())),
            ServiceType::create::<Reports>(),
        ),
        source: ServiceSource::new("business/reports.rs", 82, 9),
        label: Some("monthly"),
        input: InputSlot::new(3),
        project: project_required::<Reports>,
    });
    let token =
        LazyInjection::<Reports>::new(with_plan(&resolver, plan.clone()), InputSlot::new(3));
    // 消费者句柄共享不可变描述；owner 不保活描述，也不能成为错误来源的存储位置。
    assert_eq!(Arc::strong_count(&plan), 2);
    drop(plan);
    drop(resolver);
    assert!(weak.upgrade().is_none());
    let error = token.get().await.err().unwrap().to_string();
    assert!(error.contains("owner 已关闭"));
    assert!(error.contains("monthly-reports"));
    assert!(error.contains("business/reports.rs:82:9"));
    assert!(error.contains("延迟注入 monthly"));
}

#[tokio::test]
async fn input_preparation_is_lazy_and_concurrent_gets_share_a_typed_token() {
    let resolver = resolver();
    let slot = InputSlot::new(0);
    let mut preparation = ActivationPreparation::new(1);
    preparation
        .prepare_lazy(
            slot,
            prepare_lazy_required::<Reports>,
            Some(dependency(
                &resolver,
                project_required::<Reports>,
                InputSlot::new(0),
            )),
        )
        .unwrap();
    let (mut inputs, leases) = preparation.finish_class().unwrap();
    assert!(leases.is_empty());
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    // 普通读取不能误消费延迟载荷；失败后仍能取出原来的延迟句柄。
    assert!(inputs.take::<Reports>(slot).is_err());
    let token = inputs.take_lazy::<Reports>(slot).unwrap();
    inputs.ensure_all_consumed().unwrap();
    let (first, second) = tokio::join!(token.get(), token.get());
    let first = first.unwrap();
    assert_eq!(first.0, 42);
    assert!(std::ptr::eq(first, second.unwrap()));
    assert!(std::ptr::eq(first, token.get().await.unwrap()));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_trait_target_uses_the_selected_binding_projection() {
    fn project(
        slot: InputSlot,
        input: ErasedServiceRef,
        target: &mut ProjectionTarget<'_>,
    ) -> Result<(), ConstructionError> {
        project_bound::<Reports, dyn ReportPort>(slot, input, target, |value| value)
    }
    let resolver = resolver();
    let token = LazyInjection::<dyn ReportPort>::new(
        dependency(&resolver, project, InputSlot::new(0)),
        InputSlot::new(0),
    );
    assert_eq!(token.get().await.unwrap().count(), 42);
}

#[tokio::test]
async fn optional_lazy_targets_preserve_absence_and_trait_projection() {
    fn project(
        slot: InputSlot,
        input: ErasedServiceRef,
        target: &mut ProjectionTarget<'_>,
    ) -> Result<(), ConstructionError> {
        project_bound::<Reports, dyn ReportPort>(slot, input, target, |value| value)
    }
    let resolver = resolver();
    let mut preparation = ActivationPreparation::new(2);
    preparation
        .prepare_lazy(InputSlot::new(0), prepare_lazy_optional::<Reports>, None)
        .unwrap();
    preparation
        .prepare_lazy(
            InputSlot::new(1),
            prepare_lazy_optional::<dyn ReportPort>,
            Some(dependency(&resolver, project, InputSlot::new(1))),
        )
        .unwrap();
    let (mut inputs, leases) = preparation.finish_class().unwrap();
    assert!(leases.is_empty());
    assert!(inputs.take_optional::<Reports>(InputSlot::new(0)).is_err());
    assert!(
        inputs
            .take_optional_lazy::<Reports>(InputSlot::new(0))
            .unwrap()
            .is_none()
    );
    let token = inputs
        .take_optional_lazy::<dyn ReportPort>(InputSlot::new(1))
        .unwrap()
        .unwrap();
    assert_eq!(token.get().await.unwrap().count(), 42);
    inputs.ensure_all_consumed().unwrap();
}

#[tokio::test]
async fn failures_are_cached_per_handle() {
    let resolver = Arc::new(Resolver {
        calls: AtomicUsize::new(0),
        result: Err(ResolveError::new("target failed".to_owned())),
    });
    let token = LazyInjection::<Reports>::new(
        dependency(&resolver, project_required::<Reports>, InputSlot::new(0)),
        InputSlot::new(0),
    );
    for _ in 0..2 {
        assert!(
            token
                .get()
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("target failed")
        );
    }
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn shared_input_plans_do_not_share_occurrences_or_failed_handle_state() {
    struct OccurrenceResolver(AtomicUsize);
    impl LazyResolver for OccurrenceResolver {
        fn request(&self, provider: usize) -> Result<LazyReceiver, &'static str> {
            assert_eq!(provider, 17);
            let occurrence = self.0.fetch_add(1, Ordering::SeqCst);
            let result = if occurrence == 0 {
                Err(ResolveError::new("first occurrence failed".to_owned()))
            } else {
                Ok(DependencyLease::new(
                    ErasedService::new(Reports(occurrence as u32)),
                    vec![],
                    ReleaseDomain::new(),
                ))
            };
            let (_, receiver) = tokio::sync::watch::channel(Some(result));
            Ok(receiver)
        }
    }

    let resolver = Arc::new(OccurrenceResolver(AtomicUsize::new(0)));
    let plan = plan(project_required::<Reports>, InputSlot::new(0));
    let make_token =
        || LazyInjection::<Reports>::new(with_plan(&resolver, plan.clone()), InputSlot::new(0));
    let failed = make_token();
    let first = make_token();
    let second = make_token();
    assert_eq!(Arc::strong_count(&plan), 4);

    // 同一条图边会被多个 Transient 消费者复用。共享的是声明，不是字段的请求、
    // OnceCell 或失败记录；一个消费者失败不能阻断另一个消费者的独立 occurrence。
    assert!(
        failed
            .get()
            .await
            .err()
            .unwrap()
            .to_string()
            .contains("first occurrence failed")
    );
    let first_value = first.get().await.unwrap();
    let second_value = second.get().await.unwrap();
    assert_eq!((first_value.0, second_value.0), (1, 2));
    assert!(!std::ptr::eq(first_value, second_value));
    assert!(std::ptr::eq(first_value, first.get().await.unwrap()));
    assert!(failed.get().await.is_err());
    assert_eq!(resolver.0.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_safe_failing_projector_may_retain_a_token_without_dangling_or_double_release() {
    use std::sync::Mutex;

    struct TrackedReports(Arc<AtomicUsize>);
    impl Drop for TrackedReports {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    static RETAINED: Mutex<Option<Injection<TrackedReports>>> = Mutex::new(None);
    fn retain_and_fail(
        slot: InputSlot,
        input: ErasedServiceRef,
        _target: &mut ProjectionTarget<'_>,
    ) -> Result<(), ConstructionError> {
        // 手写 adapter 可通过现有安全准备 API 获得真实 token，然后选择拒绝交付。
        // 运行时不能假设 Err 表示业务代码没有保留该实例的 lease。
        let token = crate::activation::prepare_required::<TrackedReports>(slot, Some(input))?
            .into_required::<TrackedReports>(slot)?;
        *RETAINED.lock().unwrap() = Some(token);
        Err(ConstructionError::UnfilledSlot { slot })
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let resolver = Arc::new(Resolver {
        calls: AtomicUsize::new(0),
        result: Ok(DependencyLease::new(
            ErasedService::new(TrackedReports(drops.clone())),
            vec![],
            ReleaseDomain::new(),
        )),
    });
    let token = LazyInjection::<TrackedReports>::new(
        dependency(&resolver, retain_and_fail, InputSlot::new(0)),
        InputSlot::new(0),
    );
    for _ in 0..2 {
        assert!(
            token
                .get()
                .await
                .err()
                .unwrap()
                .to_string()
                .contains("尚未准备")
        );
    }
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    drop(resolver);
    drop(token);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let retained = RETAINED.lock().unwrap().take().unwrap();
    assert!(Arc::ptr_eq(&retained.0, &drops));
    drop(retained);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn access_rejection_is_not_cached_and_ready_values_do_not_require_wait_permission() {
    let resolver = resolver();
    let token = LazyInjection::<Reports>::new(
        dependency(&resolver, project_required::<Reports>, InputSlot::new(0)),
        InputSlot::new(0),
    );
    // 句柄只依赖协议，不感知 Tokio 的任务阶段。运行时的实际构造保护另有测试。
    let error = WAIT_REJECTED.scope((), token.get()).await.err().unwrap();
    assert!(error.to_string().contains("不允许等待"));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(token.get().await.unwrap().0, 42);
    // 已交付 token 不再等待，外部规则拒绝新的等待也不能禁止安全的缓存读取。
    assert_eq!(WAIT_REJECTED.scope((), token.get()).await.unwrap().0, 42);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_wrong_projector_cannot_create_a_forged_typed_reference() {
    let resolver = resolver();
    let token = LazyInjection::<u32>::new(
        dependency(&resolver, project_required::<Reports>, InputSlot::new(0)),
        InputSlot::new(0),
    );
    let error = token.get().await.err().unwrap();
    assert!(error.to_string().contains("类型不匹配"));
    assert!(error.to_string().contains("reports"));
}

#[test]
fn a_missing_required_lazy_dependency_fails_preparation() {
    assert!(matches!(
        prepare_lazy_required::<Reports>(InputSlot::new(0), None),
        Err(ConstructionError::RequiredDependencyAbsent { .. })
    ));
}

#[test]
fn a_deep_resolved_lazy_chain_releases_iteratively_after_runtime_exit() {
    // 刻意让实例仅通过 LazyInjection 的 lease 连成链，排除 journal 或普通输入的
    // dependencies 列表代为保活。小线程栈可暴露句柄析构中重新引入的递归 Arc 释放。
    let thread = std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(|| {
            struct Node {
                _next: Option<LazyInjection<Node>>,
                drops: Arc<AtomicUsize>,
            }
            impl Drop for Node {
                fn drop(&mut self) {
                    self.drops.fetch_add(1, Ordering::SeqCst);
                }
            }
            let drops = Arc::new(AtomicUsize::new(0));
            let runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            let top = runtime.block_on(async {
                let domain = ReleaseDomain::new();
                let mut top = DependencyLease::new(
                    ErasedService::new(Node {
                        _next: None,
                        drops: drops.clone(),
                    }),
                    vec![],
                    domain.clone(),
                );
                for _ in 0..10_000 {
                    let resolver = Arc::new(Resolver {
                        calls: AtomicUsize::new(0),
                        result: Ok(top),
                    });
                    let token = LazyInjection::<Node>::new(
                        dependency(&resolver, project_required::<Node>, InputSlot::new(0)),
                        InputSlot::new(0),
                    );
                    token.get().await.unwrap();
                    // 首次交付后只由延迟槽位保活下一个实例，不能靠 mock resolver 留存。
                    drop(resolver);
                    top = DependencyLease::new(
                        ErasedService::new(Node {
                            _next: Some(token),
                            drops: drops.clone(),
                        }),
                        vec![],
                        domain.clone(),
                    );
                }
                top
            });
            drop(runtime);
            assert_eq!(drops.load(Ordering::SeqCst), 0);
            drop(top);
            assert_eq!(drops.load(Ordering::SeqCst), 10_001);
        })
        .unwrap();
    thread.join().unwrap();
}
