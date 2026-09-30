//! 一个延迟消费槽位的请求状态。状态属于字段，绝不属于第一个等待它的 future。
//!
//! 槽位只允许提交一次；协调器收到请求后负责初始化和发布。watch 接收端一直由槽位
//! 保存，因此取消、重新等待或并发等待都不会再次创建 Transient occurrence。

use std::{
    future::Future,
    pin::Pin,
    sync::atomic::Ordering,
    sync::{Arc, Mutex, Weak},
};

use tokio::sync::{mpsc, watch};

use super::{Resolution, handle::Command, owner::OwnerData};
use crate::{
    activation::lazy::{LazyDependency, LazyResolver},
    error::ResolveError,
    graph::{CompiledDependency, CompiledNode},
    service::{ServiceIdentifier, ServiceSource},
};

pub(super) struct LazySlot {
    owner: Weak<OwnerData>,
    commands: mpsc::WeakUnboundedSender<Command>,
    provider: usize,
    consumer: ServiceIdentifier,
    source: ServiceSource,
    label: Option<&'static str>,
    // None 是尚未请求，不是初始化失败。Some 内同时保存进行中和最终结果。
    receiver: Mutex<Option<watch::Receiver<Option<Resolution>>>>,
}

impl LazySlot {
    pub(super) fn dependency(
        owner: &Arc<OwnerData>,
        commands: mpsc::WeakUnboundedSender<Command>,
        provider: usize,
        consumer: &CompiledNode,
        dependency: &CompiledDependency,
    ) -> LazyDependency {
        LazyDependency {
            resolver: Arc::new(Self {
                owner: Arc::downgrade(owner),
                commands,
                provider,
                consumer: consumer.identifier.clone(),
                source: consumer.common.source,
                label: dependency.label,
                receiver: Mutex::new(None),
            }),
            preparer: dependency.prepare,
            optional: dependency.optional,
        }
    }

    /// 锁只保护“建立唯一请求”这个同步步骤，不持锁等待，也不持有 owner 强引用跨 await。
    fn subscribe(&self) -> Result<watch::Receiver<Option<Resolution>>, ResolveError> {
        let mut state = self
            .receiver
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(receiver) = &*state {
            return Ok(receiver.clone());
        }
        let closed = || self.error("服务 owner 已关闭或正在关闭，无法首次获取延迟依赖".into());
        let owner = self.owner.upgrade().ok_or_else(closed)?;
        if owner.status.load(Ordering::Acquire) != super::owner::OPEN {
            return Err(closed());
        }
        let commands = self.commands.upgrade().ok_or_else(closed)?;
        let (waiter, receiver) = watch::channel(None);
        commands
            .send(Command::ResolveLazy {
                owner: owner.id,
                provider: self.provider,
                waiter,
            })
            .map_err(|_| closed())?;
        *state = Some(receiver.clone());
        Ok(receiver)
    }
}

impl LazyResolver for LazySlot {
    fn resolve(&self) -> Pin<Box<dyn Future<Output = Resolution> + Send + '_>> {
        Box::pin(async move {
            let mut receiver = self.subscribe()?;
            loop {
                // 必须在 await 之前释放 watch 的读锁，防止完成者等待当前读取者。
                let result = receiver.borrow().clone();
                if let Some(result) = result {
                    return result.map_err(|error| {
                        ResolveError::dependency(&self.consumer, self.source, error)
                    });
                }
                receiver
                    .changed()
                    .await
                    .map_err(|_| self.error("Tokio 协调器已停止，延迟初始化未完成".into()))?;
            }
        })
    }

    fn error(&self, message: String) -> ResolveError {
        let label = self.label.unwrap_or("未命名字段");
        ResolveError::construction(
            &self.consumer,
            self.source,
            format!("延迟注入字段 {label}：{message}"),
        )
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime/lazy.rs"]
mod tests;
