//! 执行单个已就绪节点的构造或单个已发布实例的清理。
//!
//! worker 不读取缓存、不请求依赖，也不决定 owner 状态。它接收完整输入并返回结果；
//! 发布顺序、失败传播和逐 owner 串行 cleanup 均由中央协调器决定。

use super::{Resolution, owner::Published};
use crate::{
    activation::{ActivationPreparation, DependencyLease, ReleaseDomain},
    error::ResolveError,
    graph::{Constructor, ValidatedGraph},
    registration::provider::FactoryInvoker,
};
use std::{
    any::Any,
    future::poll_fn,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
    task::Poll,
};

pub(super) async fn activate(
    graph: Arc<ValidatedGraph>,
    provider: usize,
    inputs: Vec<Option<DependencyLease>>,
    lazy_inputs: Vec<Option<crate::activation::lazy::LazyDependency>>,
    domain: Arc<ReleaseDomain>,
) -> Resolution {
    let node = &graph.nodes[provider];
    let convert = |error: crate::activation::ConstructionError| {
        ResolveError::construction(&node.identifier, node.common.source, error.to_string())
    };
    // 每个 worker 只准备当前节点的输入，依赖实例已由调度器构造并以强 lease 传入。
    // Preparation 对写入失败负责回滚，factory frame 则持有跨 await 的真实借用对象。
    let mut preparation = ActivationPreparation::new(node.dependencies.len());
    for ((dependency, input), lazy_input) in node.dependencies.iter().zip(inputs).zip(lazy_inputs) {
        if let Some(preparer) = dependency.lazy {
            preparation
                .prepare_lazy(dependency.slot, preparer, lazy_input)
                .map_err(convert)?;
        } else {
            preparation
                .prepare(dependency.slot, dependency.prepare, input)
                .map_err(convert)?;
        }
    }
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
