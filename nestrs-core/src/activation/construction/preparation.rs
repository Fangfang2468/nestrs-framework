//! 已选定依赖到固定构造输入的事务式准备。
//!
//! 一个依赖槽位依次经历“生成完整令牌 → 写入空槽位 → 收纳令牌的 lease”。前两步任一
//! 失败都由 Rust 正常析构临时值，既不留下半填槽位，也不把失败输入加入依赖保活列表。
//! 所有槽位就绪后才交给消费侧；必选缺失、可选缺席和未填槽位始终分别处理。

use super::{
    ConstructionError, ConstructionInputs, FactoryLeaseFrame, InputPreparer, InputSlot,
    LazyInputPreparer, PreparedInput,
};
use crate::activation::{DependencyLease, lazy::LazyDependency};

/// 一次构造的准备事务；与固定槽位缓冲区共置，集中维护输入和 lease 的提交顺序。
pub(crate) struct ActivationPreparation {
    buffer: InputBuffer,
    dependencies: Vec<DependencyLease>,
}

impl ActivationPreparation {
    pub(crate) fn new(slot_count: usize) -> Self {
        Self {
            buffer: InputBuffer::new(slot_count),
            dependencies: Vec::with_capacity(slot_count),
        }
    }

    pub(crate) fn prepare(
        &mut self,
        slot: InputSlot,
        preparer: InputPreparer,
        input: Option<DependencyLease>,
    ) -> Result<(), ConstructionError> {
        let prepared = preparer(slot, input.as_ref().map(DependencyLease::erased_ref))?;
        // 保留实际返回令牌的 owner，而不是直接克隆 input：安全的手写 preparer 也可能
        // 返回此前准备的另一实例。先取得临时 lease，但仅在插入成功后把它提交到列表；
        // 插入失败时临时 lease 和被拒绝的令牌一并释放。
        let dependency = prepared.dependency();
        self.buffer.insert(slot, prepared)?;
        if let Some(dependency) = dependency {
            self.dependencies.push(dependency);
        }
        Ok(())
    }

    /// 延迟槽位只提交完整句柄，不把尚不存在的目标加入构造帧的依赖 lease。
    /// 插入失败时句柄按正常 Rust 所有权回滚；未调用 resolver，故没有构造副作用。
    pub(crate) fn prepare_lazy(
        &mut self,
        slot: InputSlot,
        preparer: LazyInputPreparer,
        input: Option<LazyDependency>,
    ) -> Result<(), ConstructionError> {
        let prepared = preparer(slot, input)?;
        self.buffer.insert(slot, prepared)
    }

    pub(crate) fn finish_class(
        self,
    ) -> Result<(ConstructionInputs, Vec<DependencyLease>), ConstructionError> {
        Ok((self.buffer.finish()?, self.dependencies))
    }

    pub(crate) fn finish_factory(self) -> Result<FactoryLeaseFrame, ConstructionError> {
        let (inputs, dependencies) = self.finish_class()?;
        Ok(FactoryLeaseFrame::new(inputs, dependencies))
    }
}

/// 准备阶段专用的固定长度写入器；构造适配器无法获得写入权限。
///
/// buffer 不会按槽位号扩容：每个 slot 都必须在 [`Self::finish`] 前恰好写入一次。
pub(super) struct InputBuffer {
    slots: Vec<BufferSlot>,
}

enum BufferSlot {
    Empty,
    Ready(PreparedInput),
}

impl InputBuffer {
    pub(super) fn new(slot_count: usize) -> Self {
        Self {
            slots: std::iter::repeat_with(|| BufferSlot::Empty)
                .take(slot_count)
                .collect(),
        }
    }

    /// 写入一个已经准备好的输入。
    ///
    /// 写入失败时 `input` 会直接被销毁，buffer 本身不发生变化。
    pub(super) fn insert(
        &mut self,
        slot: InputSlot,
        input: PreparedInput,
    ) -> Result<(), ConstructionError> {
        let slot_count = self.slots.len();
        let Some(target) = self.slots.get_mut(slot.index()) else {
            return Err(ConstructionError::SlotOutOfBounds { slot, slot_count });
        };

        if matches!(target, BufferSlot::Ready(_)) {
            return Err(ConstructionError::SlotAlreadyPrepared { slot });
        }

        *target = BufferSlot::Ready(input);
        Ok(())
    }

    /// 验证每个固定槽位均已准备，并转换为只能被 adapter 消费的输入。
    pub(super) fn finish(self) -> Result<ConstructionInputs, ConstructionError> {
        let mut prepared = Vec::with_capacity(self.slots.len());

        for (index, slot) in self.slots.into_iter().enumerate() {
            let BufferSlot::Ready(input) = slot else {
                return Err(ConstructionError::UnfilledSlot {
                    slot: InputSlot::new(index),
                });
            };

            prepared.push(input);
        }

        Ok(ConstructionInputs::from_prepared(prepared))
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/preparation.rs"]
mod tests;
