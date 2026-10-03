//! 执行单个已就绪节点的构造或单个已发布实例的清理。
//!
//! worker 不读取缓存、不请求依赖，也不决定 owner 状态。它接收完整输入并返回结果；
//! 发布顺序、失败传播和逐 owner 串行 cleanup 均由中央协调器决定。

use super::{Resolution, owner::Published};
use crate::{
    activation::{
        ActivationPreparation, DependencyLease, ReleaseDomain, adapter::FactoryInvoker,
        deferred::LazyResolver,
    },
    error::ResolveError,
    graph::{AbsentInput, Constructor, DependencyInput, ValidatedGraph},
};
use std::{
    any::Any,
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Weak},
    task::Poll,
};

pub(super) async fn activate(
    graph: Arc<ValidatedGraph>,
    provider: usize,
    inputs: Vec<Option<DependencyLease>>,
    resolver: Weak<dyn LazyResolver>,
    domain: Arc<ReleaseDomain>,
) -> Resolution {
    let node = &graph.nodes[provider];
    let convert = |error: crate::activation::ConstructionError| {
        ResolveError::construction(&node.identifier, node.common.source, error.to_string())
    };
    // 每个 worker 只准备当前节点的输入，依赖实例已由调度器构造并以强 lease 传入。
    // Preparation 对写入失败负责回滚，factory frame 则持有跨 await 的真实借用对象。
    let mut preparation = ActivationPreparation::new(node.dependencies.len());
    for (dependency, input) in node.dependencies.iter().zip(inputs) {
        // 计划已经决定完整交付形态。缺席分支使用准确类型的 None；只有立即输入
        // 消费已就绪实例，只有实际延迟目标才接收关联 owner 的请求句柄。
        match &dependency.input {
            DependencyInput::Absent(AbsentInput::Immediate(prepare)) => {
                preparation.prepare(dependency.slot, *prepare, None)
            }
            DependencyInput::Absent(AbsentInput::Lazy(prepare)) => {
                preparation.prepare_lazy(dependency.slot, *prepare, None)
            }
            DependencyInput::Immediate { prepare, .. } => {
                preparation.prepare(dependency.slot, *prepare, input)
            }
            DependencyInput::Lazy { plan, prepare } => {
                // 固定描述与实际 owner 只在交付此槽位时组合，不再维护另一条并行输入数组。
                // 包装弱能力不提交目标请求；每个字段仍独占后续的接收端与类型化结果。
                let lazy_input = super::lazy::dependency(&resolver, plan.clone());
                preparation.prepare_lazy(dependency.slot, *prepare, Some(lazy_input))
            }
        }
        .map_err(convert)?;
    }
    // 字段已各自保存弱能力，worker 不需要在用户构造 future 中继续持有它。
    drop(resolver);
    let (service, dependencies) = match node.constructor {
        Constructor::Class(constructor) => {
            let (inputs, dependencies) = preparation.finish_class().map_err(convert)?;
            (constructor(inputs).map_err(convert)?, dependencies)
        }
        Constructor::Factory(invoker) => {
            let mut frame = preparation.finish_factory().map_err(convert)?;
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

pub(super) async fn cleanup(entry: Published, graph: Arc<ValidatedGraph>) -> Vec<String> {
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
            Err(payload) => errors.push(describe("cleanup panic", panic_message(payload.as_ref()))),
            Ok(mut future) => {
                let outcome = poll_fn(|cx| {
                    match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
                        Ok(Poll::Pending) => Poll::Pending,
                        Ok(Poll::Ready(())) => Poll::Ready(Ok(())),
                        Err(payload) => Poll::Ready(Err(panic_message(payload.as_ref()))),
                    }
                })
                .await;
                if let Err(detail) = outcome {
                    errors.push(describe("cleanup panic", detail));
                }
                if let Err(payload) = catch_unwind(AssertUnwindSafe(|| drop(future))) {
                    errors.push(describe(
                        "cleanup future Drop panic",
                        panic_message(payload.as_ref()),
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

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|message| (*message).to_owned())
        })
        .unwrap_or_else(|| "未提供字符串 panic 信息".to_owned())
}
