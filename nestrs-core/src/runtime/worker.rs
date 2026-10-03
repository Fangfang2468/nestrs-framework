//! 执行单个已就绪节点的构造或单个已发布实例的清理。
//!
//! worker 不读取缓存、不请求依赖，也不决定 owner 状态。它接收完整输入并返回结果；
//! 发布顺序、失败传播和逐 owner 串行 cleanup 均由中央协调器决定。

use super::{Resolution, lazy::ActivationContext, owner::Published};
use crate::{
    activation::{
        ConstructionError, ConstructionInput, ConstructionInputs, DependencyLease, InputSlot,
        ReleaseDomain, adapter::FactoryInvoker, construction::FactoryLeaseFrame,
        deferred::LazyResolver,
    },
    error::ResolveError,
    graph::{Constructor, DependencyInput, ValidatedGraph},
    panic_payload::PanicPayload,
};
use std::{
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Weak},
    task::Poll,
};

/// 一次就绪构造的执行上下文。调度器移交输入后，由 worker 独占准备与调用过程。
pub(super) struct ActivationWorker {
    graph: Arc<ValidatedGraph>,
    provider: usize,
    inputs: Vec<Option<DependencyLease>>,
    context: ActivationContext,
    domain: Arc<ReleaseDomain>,
}

impl ActivationWorker {
    pub(super) fn new(
        graph: Arc<ValidatedGraph>,
        provider: usize,
        inputs: Vec<Option<DependencyLease>>,
        resolver: Weak<dyn LazyResolver>,
        domain: Arc<ReleaseDomain>,
    ) -> Self {
        Self {
            graph,
            provider,
            inputs,
            context: ActivationContext::new(resolver),
            domain,
        }
    }

    /// 激活标记覆盖输入准备和完整业务 future，调用者无需自行安装 task-local。
    pub(super) async fn run(self) -> Resolution {
        ActivationContext::run(self.construct()).await
    }

    async fn construct(self) -> Resolution {
        let Self {
            graph,
            provider,
            inputs,
            context,
            domain,
        } = self;
        let node = &graph.nodes[provider];
        let convert = |error: crate::activation::ConstructionError| {
            ResolveError::construction(&node.identifier, node.common.source, error.to_string())
        };
        // 调度器已经构造 Immediate 目标。这里只组合已选执行信息和真实实例凭证，
        // 不调用逐参数准备回调；生成 adapter 会一次读取全部 typed 输入再执行业务代码。
        let slot_count = node.dependencies.len();
        if inputs.len() < slot_count {
            return Err(convert(ConstructionError::UnfilledSlot {
                slot: InputSlot::new(inputs.len()),
            }));
        }
        if inputs.len() > slot_count {
            return Err(convert(ConstructionError::SlotOutOfBounds {
                slot: InputSlot::new(slot_count),
                slot_count,
            }));
        }
        // 已知准确槽位数，不能用 Result<Vec<_>> 收集丢失 size_hint 后按最小容量增长。
        // 输入记录含类型及执行能力，短参数列表的多余容量会抵消去除逐参数 Box 的收益。
        let mut selected = Vec::with_capacity(slot_count);
        for (index, (dependency, input)) in node.dependencies.iter().zip(inputs).enumerate() {
            let slot = InputSlot::new(index);
            if dependency.slot != slot {
                return Err(convert(ConstructionError::InputSlotMismatch {
                    slot,
                    actual: dependency.slot,
                }));
            }
            let service_type = dependency.requested.service_type;
            let kind = dependency.kind();
            let input = match &dependency.input {
                DependencyInput::Immediate { project, .. } => {
                    let lease = input.ok_or_else(|| {
                        convert(ConstructionError::RequiredDependencyAbsent { slot })
                    })?;
                    ConstructionInput::immediate(service_type, kind, lease, *project)
                }
                DependencyInput::Absent(_) => {
                    if input.is_some() {
                        return Err(convert(ConstructionError::UnexpectedDependencyPresent {
                            slot,
                        }));
                    }
                    ConstructionInput::absent(service_type, kind)
                }
                DependencyInput::Lazy { plan } => {
                    if input.is_some() {
                        return Err(convert(ConstructionError::UnexpectedDependencyPresent {
                            slot,
                        }));
                    }
                    // 共享描述与实际 owner 在本次槽位组合；不提交目标请求或收纳强 lease。
                    ConstructionInput::lazy(service_type, kind, context.dependency(plan.clone()))
                }
            };
            selected.push(input);
        }
        let inputs = ConstructionInputs::new(selected).map_err(convert)?;
        // 延迟输入已经保存弱能力，worker 不在用户构造 future 中额外持有上下文。
        drop(context);
        let (service, dependencies) = match node.constructor {
            Constructor::Class(constructor) => {
                let dependencies = inputs.dependency_leases();
                (constructor(inputs).map_err(convert)?, dependencies)
            }
            Constructor::Factory(invoker) => {
                let mut frame = FactoryLeaseFrame::new(inputs);
                let service = match invoker {
                    FactoryInvoker::Sync(constructor) => {
                        constructor(frame.inputs()).map_err(convert)?
                    }
                    FactoryInvoker::Async(constructor) => {
                        constructor(frame.inputs()).await.map_err(convert)?
                    }
                };
                (service, frame.into_dependencies())
            }
        };
        // 在发布前确认类型身份。任何失败都会释放未发布的结果，不允许错误类型进入缓存或 journal。
        if service.service_type() != node.identifier.service_type {
            return Err(ResolveError::construction(
                &node.identifier,
                node.common.source,
                format!("构造器返回了不匹配的服务类型 {:?}", service.service_type()),
            ));
        }
        Ok(DependencyLease::new(service, dependencies, domain))
    }
}

/// 一个已发布实例的关闭上下文，包含 hook 及本次释放触发的析构工作。
pub(super) struct CleanupWorker {
    entry: Published,
    graph: Arc<ValidatedGraph>,
}

impl CleanupWorker {
    pub(super) fn new(entry: Published, graph: Arc<ValidatedGraph>) -> Self {
        Self { entry, graph }
    }

    pub(super) async fn run(self) -> Vec<String> {
        let Self { entry, graph } = self;
        let node = &graph.nodes[entry.provider];
        let mut errors = Vec::new();
        let describe = |stage: &str, detail: String| {
            format!(
                "{:?}（{:?}）{stage}：{detail}",
                node.identifier, node.common.source,
            )
        };
        // hook 的创建、每次 poll 和 future 析构都可能 panic，分别记录才能继续清理剩余实例。
        // 只有整个 hook 结束且其 future 被释放后，才释放当前实例；协调器随后才推进下一个 cleanup。
        if let Some(hook) = node.common.cleanup {
            match catch_unwind(AssertUnwindSafe(hook)) {
                Err(payload) => errors.push(describe(
                    "cleanup panic",
                    PanicPayload::new(payload).into_message(),
                )),
                Ok(mut future) => {
                    let outcome = poll_fn(|cx| {
                        match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
                            Ok(Poll::Pending) => Poll::Pending,
                            Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
                            Err(payload) => {
                                Poll::Ready(Err(PanicPayload::new(payload).into_message()))
                            }
                        }
                    })
                    .await;
                    if let Err(detail) = outcome {
                        errors.push(describe("cleanup panic", detail));
                    }
                    if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(future))) {
                        errors.push(describe(
                            "cleanup future Drop panic",
                            PanicPayload::new(payload).into_message(),
                        ));
                    }
                }
            }
        }
        // 逃逸 lease 可以延长内存存活，不能阻塞逻辑关闭；只有当前释放触发的析构工作需要等待。
        if let Some(completion) = entry.lease.release_tracked() {
            for detail in completion.await {
                errors.push(describe("service Drop panic", detail));
            }
        }
        errors
    }
}
