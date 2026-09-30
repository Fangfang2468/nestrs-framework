use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use super::{IN_ACTIVATION, LazyDependency, LazyInjection, LazyResolver};
use crate::{
    ResolveError,
    activation::{
        ActivationPreparation, ConstructionError, DependencyLease, ErasedService, ErasedServiceRef,
        Injection, InputPreparer, InputSlot, PreparedInput, ReleaseDomain, prepare_bound_optional,
        prepare_bound_required, prepare_lazy_optional, prepare_lazy_required, prepare_required,
    },
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
    fn resolve(
        &self,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<DependencyLease, ResolveError>> + Send + '_>>
    {
        Box::pin(async {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            self.result.clone()
        })
    }

    fn error(&self, message: String) -> ResolveError {
        ResolveError::new(format!("lazy field: {message}"))
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

fn dependency(resolver: Arc<Resolver>, preparer: InputPreparer, optional: bool) -> LazyDependency {
    LazyDependency {
        resolver,
        preparer,
        optional,
    }
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
async fn input_preparation_is_lazy_and_concurrent_gets_share_a_typed_token() {
    let resolver = resolver();
    let slot = InputSlot::new(0);
    let mut preparation = ActivationPreparation::new(1);
    preparation
        .prepare_lazy(
            slot,
            prepare_lazy_required::<Reports>,
            Some(dependency(
                resolver.clone(),
                prepare_required::<Reports>,
                false,
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
        input: Option<ErasedServiceRef>,
    ) -> Result<PreparedInput, ConstructionError> {
        prepare_bound_required::<Reports, dyn ReportPort>(slot, input, |value| value)
    }
    let token = LazyInjection::<dyn ReportPort>::new(
        dependency(resolver(), project, false),
        InputSlot::new(0),
    );
    assert_eq!(token.get().await.unwrap().count(), 42);
}

#[tokio::test]
async fn optional_lazy_targets_preserve_absence_and_trait_projection() {
    fn project(
        slot: InputSlot,
        input: Option<ErasedServiceRef>,
    ) -> Result<PreparedInput, ConstructionError> {
        prepare_bound_optional::<Reports, dyn ReportPort>(slot, input, |value| value)
    }
    let mut preparation = ActivationPreparation::new(2);
    preparation
        .prepare_lazy(InputSlot::new(0), prepare_lazy_optional::<Reports>, None)
        .unwrap();
    preparation
        .prepare_lazy(
            InputSlot::new(1),
            prepare_lazy_optional::<dyn ReportPort>,
            Some(dependency(resolver(), project, true)),
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
        dependency(resolver.clone(), prepare_required::<Reports>, false),
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
async fn activation_guard_rejects_unready_access_without_poisoning_the_handle() {
    let resolver = resolver();
    let token = LazyInjection::<Reports>::new(
        dependency(resolver.clone(), prepare_required::<Reports>, false),
        InputSlot::new(0),
    );
    let error = IN_ACTIVATION.scope((), token.get()).await.err().unwrap();
    assert!(error.to_string().contains("构造期间"));
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    assert_eq!(token.get().await.unwrap().0, 42);
    // 已有真实 token 的读取不会等待新 worker，构造阶段可以安全读取。
    assert_eq!(IN_ACTIVATION.scope((), token.get()).await.unwrap().0, 42);
}

#[tokio::test]
async fn a_wrong_preparer_cannot_create_a_forged_typed_reference() {
    let token = LazyInjection::<u32>::new(
        dependency(resolver(), prepare_required::<Reports>, false),
        InputSlot::new(0),
    );
    let error = token.get().await.err().unwrap();
    assert!(error.to_string().contains("类型不匹配"));
    assert!(error.to_string().contains("lazy field"));
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
                        dependency(resolver, prepare_required::<Node>, false),
                        InputSlot::new(0),
                    );
                    token.get().await.unwrap();
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
