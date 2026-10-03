//! 稳定实例及其依赖闭包的强所有权。
//!
//! 一个实例记录只有一个 [`ErasedService`]，所有注入令牌和 owner 记录共享它的 lease。
//! 最后一个 lease 被释放时，把整个载荷交给 [`ReleaseDomain`]；这里不直接递归销毁
//! 依赖。异步 cleanup 属于运行时的逻辑关闭流程，本模块只负责最终的同步内存释放。

use std::{ptr::NonNull, sync::Arc};

use super::{
    ErasedService, ErasedServiceRef,
    release::{ReleaseCompletion, ReleaseDomain},
};
use crate::service::{Injectable, ServiceType};

/// 整体交给释放队列的载荷；字段顺序本身就是析构契约。
pub(super) struct InstancePayload {
    // Rust 按声明顺序析构字段：先销毁消费者，再释放依赖边。
    // 即使消费者的 Drop 展开栈，其依赖也必须在消费者析构期间保持有效。
    /// 消费者本体，按字段析构顺序先于其普通依赖释放。
    service: ErasedService,

    /// 构造时保存的普通输入 lease，覆盖消费者析构期间的依赖存活。
    _dependencies: Vec<DependencyLease>,
}

impl InstancePayload {
    /// 释放器只需服务名来记录析构 panic，不需要获得服务值或可变访问。
    pub(super) fn service_name(&self) -> &'static str {
        self.service.service_type().name
    }
}

/// `payload` 只会在最后一个 lease 的释放路径被取走。
struct InstanceRecord {
    /// 仅由最后一个 lease 的释放路径取走的服务与依赖。
    payload: Option<InstancePayload>,

    /// 实例最终释放使用的同步域，独立于 Tokio 任务存活。
    domain: Arc<ReleaseDomain>,
}

impl Drop for InstanceRecord {
    /// 将最后一个实例记录的载荷移交释放域，避免递归销毁依赖。
    fn drop(&mut self) {
        if let Some(payload) = self.payload.take() {
            self.domain.release(payload);
        }
    }
}

/// 保持服务地址及其依赖闭包存活的内部所有权凭证。
#[derive(Clone)]
pub(crate) struct DependencyLease(Arc<InstanceRecord>);

impl DependencyLease {
    /// 将服务及普通依赖固定在共享记录中，建立首个强 lease。
    pub(crate) fn new(
        service: ErasedService,
        dependencies: Vec<Self>,
        domain: Arc<ReleaseDomain>,
    ) -> Self {
        Self(Arc::new(InstanceRecord {
            payload: Some(InstancePayload {
                service,
                _dependencies: dependencies,
            }),
            domain,
        }))
    }

    /// 仅最后一个 lease 启动可等待的析构；否则立即返回 `None`。
    ///
    /// 逃逸的注入令牌可以延长内存存活期，但不能让 owner 的逻辑关闭等待业务归还它。
    /// `Arc::into_inner` 保证并发释放中至多一个调用取得记录的所有权。
    pub(crate) fn release_tracked(self) -> Option<ReleaseCompletion> {
        Arc::into_inner(self.0).map(|mut record| {
            let payload = record
                .payload
                .take()
                .expect("最后一个 lease 必须仍持有实例载荷");
            record.domain.release_tracked(payload)
        })
    }

    /// 借用仍由当前 lease 保活的服务值；已存活记录必须保留载荷。
    fn service(&self) -> &ErasedService {
        &self
            .0
            .payload
            .as_ref()
            .expect("仍持有 lease 的实例不能处于析构中")
            .service
    }

    /// 读取本实例 concrete 值的准确类型身份。
    pub(crate) fn service_type(&self) -> ServiceType {
        self.service().service_type()
    }

    /// 克隆当前 lease，供擦除投影过程保活同一个实例。
    pub(crate) fn erased_ref(&self) -> ErasedServiceRef {
        ErasedServiceRef::new(self.clone())
    }

    /// 比较实例记录身份，而非仅比较服务类型或值。
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// 从已固定实例的当前共享借用恢复准确类型地址。
    pub(crate) fn pointer<T>(&self) -> Option<NonNull<T>>
    where
        T: Injectable + ?Sized,
    {
        // 从 Arc 记录内不再移动的 envelope 恢复地址。移动/克隆 lease 不移动服务 Box；
        // 只有最后一个 lease 释放时才取走载荷，此时已没有可合法解引用该地址的持有者。
        self.service().pointer::<T>()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/activation/instance.rs"]
mod tests;
