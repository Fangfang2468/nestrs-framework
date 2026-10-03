//! 已选输入的直接类型化交付，不存放逐参数装箱的中间令牌。
//!
//! [`ConstructionInputs`] 拥有固定槽位里的实例凭证或延迟请求描述。生成的 adapter
//! 用准确的业务类型读取输入：普通依赖由既定 projector 直接写入栈上令牌，延迟依赖
//! 只交付句柄。形态、类型和投影全部成功后才消费槽位；失败不会改变原输入。

use std::mem;

use crate::{
    activation::{DependencyLease, Injection, LazyInjection},
    service::{Injectable, ServiceType},
};

use super::{ConstructionError, InputSlot, LazyDependency, ProjectionTarget, ServiceProjector};

/// 一个输入的准确交付形态；与槽位是否已消费分别记录。
///
/// 编译器在描述中固定此值。即使 optional 缺席或 lazy 尚未请求目标，也必须先检查
/// 形态和服务类型，不能把不同类型的 None 或尚未初始化的句柄相互替换。
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// 必须交付已构造实例的普通输入。
    Required,

    /// 交付普通实例令牌或已确认的缺席值。
    Optional,

    /// 必须交付能够首次获取目标的延迟句柄。
    LazyRequired,

    /// 交付延迟句柄或已确认的缺席值。
    LazyOptional,
}

impl InputKind {
    /// 判断该交付形态是否允许冻结计划选择缺席。
    pub(crate) fn is_optional(self) -> bool {
        matches!(self, Self::Optional | Self::LazyOptional)
    }

    /// 判断该槽位是否交付延迟句柄，而非已构造实例。
    pub(crate) fn is_lazy(self) -> bool {
        matches!(self, Self::LazyRequired | Self::LazyOptional)
    }

    /// 将期望的交付形态转换为带输入位置的协议错误。
    fn expected_error(self, slot: InputSlot) -> ConstructionError {
        match self {
            Self::Required => ConstructionError::RequiredInputExpected { slot },
            Self::Optional => ConstructionError::OptionalInputExpected { slot },
            Self::LazyRequired => ConstructionError::LazyRequiredInputExpected { slot },
            Self::LazyOptional => ConstructionError::LazyOptionalInputExpected { slot },
        }
    }
}

/// 一个槽位已经选定的执行数据；不包含候选查找、缓存或图分析能力。
///
/// 只有 runtime 能提供真实 lease 或当前 owner 的弱请求能力。创建整个输入集合时
/// 验证数据与交付形态一致，之后 adapter 只能读取，不能替换来源。
pub(crate) struct ConstructionInput {
    /// 该值或输入的准确 Rust 类型身份，用于交付前核对。
    service_type: ServiceType,

    /// 固定的必选、可选、普通或延迟交付形态。
    kind: InputKind,

    /// 与交付形态对应的真实执行数据。
    source: InputSource,
}

/// 一个输入实际拥有的数据：缺席、就绪实例或延迟请求上下文。
enum InputSource {
    /// 冻结选择确认该可选输入没有目标。
    Absent,

    /// 持有就绪实例及已选投影，可直接交付普通令牌。
    Immediate {
        /// 保活已选真实实例的强所有权凭证。
        lease: DependencyLease,

        /// 已选实例的准确类型投影，不重新选择目标。
        project: ServiceProjector,
    },

    /// 只保存首次请求目标所需的计划与 owner 能力。
    Lazy(LazyDependency),
}

impl ConstructionInput {
    /// 记录已经确认缺席的可选输入；完整集合建立时再核对形态。
    pub(crate) fn absent(service_type: ServiceType, kind: InputKind) -> Self {
        Self {
            service_type,
            kind,
            source: InputSource::Absent,
        }
    }

    /// 记录就绪实例及其已选投影，保留真实 lease 到读取成功。
    pub(crate) fn immediate(
        service_type: ServiceType,
        kind: InputKind,
        lease: DependencyLease,
        project: ServiceProjector,
    ) -> Self {
        Self {
            service_type,
            kind,
            source: InputSource::Immediate { lease, project },
        }
    }

    /// 记录延迟输入的计划与 owner 请求能力，不在这里请求目标。
    pub(crate) fn lazy(
        service_type: ServiceType,
        kind: InputKind,
        dependency: LazyDependency,
    ) -> Self {
        Self {
            service_type,
            kind,
            source: InputSource::Lazy(dependency),
        }
    }

    /// 检查缺席、立即或延迟来源是否符合交付形态及固定槽位。
    fn validate_source(&self, slot: InputSlot) -> Result<(), ConstructionError> {
        match &self.source {
            InputSource::Absent if !self.kind.is_optional() => {
                Err(ConstructionError::RequiredDependencyAbsent { slot })
            }
            InputSource::Immediate { .. } if self.kind.is_lazy() => {
                Err(self.kind.expected_error(slot))
            }
            InputSource::Lazy(_) if !self.kind.is_lazy() => Err(self.kind.expected_error(slot)),
            InputSource::Lazy(dependency) if dependency.plan.input != slot => {
                Err(ConstructionError::InputSlotMismatch {
                    slot,
                    actual: dependency.plan.input,
                })
            }
            _ => Ok(()),
        }
    }

    /// 按准确 Rust 类型和输入形态检查读取请求，不消费槽位。
    fn validate<T: Injectable + ?Sized>(
        &self,
        slot: InputSlot,
        kind: InputKind,
    ) -> Result<(), ConstructionError> {
        if self.kind != kind {
            return Err(kind.expected_error(slot));
        }
        if self.service_type != ServiceType::create::<T>() {
            return Err(ConstructionError::InputTypeMismatch {
                slot,
                expected: std::any::type_name::<T>(),
                actual: self.service_type.name,
            });
        }
        Ok(())
    }

    /// 从现有来源投影令牌；失败时原槽位仍保留其 lease。
    fn project<T: Injectable + ?Sized>(
        &self,
        slot: InputSlot,
    ) -> Result<Option<Injection<T>>, ConstructionError> {
        match &self.source {
            InputSource::Absent => Ok(None),
            InputSource::Immediate { lease, project } => {
                // 原输入先保留自己的 lease。只有准确投影成功，调用者才消费槽位；
                // projector 的临时失败不会丢失输入，也不能替换 frame 保活的实例。
                ProjectionTarget::project(slot, lease.clone(), *project).map(Some)
            }
            InputSource::Lazy(_) => unreachable!("普通输入必须先完成形态检查"),
        }
    }
}

/// 已完成来源验证、仅供生成构造 adapter 消费的固定输入集合。
///
/// 每个槽位只能读取一次。adapter 必须先把全部输入取到准确类型的局部变量并调用
/// [`Self::ensure_all_consumed`]，随后才执行用户 constructor、factory、Default 或 value。
#[doc(hidden)]
pub struct ConstructionInputs {
    /// 逐槽保存可用或已消费状态，不合并重复类型输入。
    slots: Vec<ConsumptionSlot>,
}

/// 区分仍可读取的输入与已成功移交的槽位，防止重复消费。
enum ConsumptionSlot {
    /// 来源已验证且仍未被 adapter 消费的输入。
    Available(ConstructionInput),

    /// 已经成功交付，任何重复读取均应拒绝。
    Consumed,
}

impl ConstructionInputs {
    /// 零依赖 adapter 的合法输入。
    pub fn empty() -> Self {
        Self { slots: Vec::new() }
    }

    /// 一次验证完整输入，不暴露可留下半填槽位的写入器。
    ///
    /// 失败时整个输入集合按 Rust 所有权释放；不会执行 projector、用户构造或 lazy 请求。
    pub(crate) fn new(inputs: Vec<ConstructionInput>) -> Result<Self, ConstructionError> {
        for (index, input) in inputs.iter().enumerate() {
            input.validate_source(InputSlot::new(index))?;
        }
        Ok(Self {
            slots: inputs.into_iter().map(ConsumptionSlot::Available).collect(),
        })
    }

    /// 从准确输入来源派生保活集合，供实例或 factory frame 持有。
    ///
    /// 只能在 adapter 消费前调用。普通投影随后必须保留同一 lease 身份；lazy 输入尚未
    /// 取得目标 lease，不加入集合，即使其他请求已构造目标。重复依赖仍逐槽保留，
    /// 不按静态图的去重边替代真实参数。
    pub(crate) fn dependency_leases(&self) -> Vec<DependencyLease> {
        // 该数组随发布实例存活到释放；filter_map().collect() 对单参数按容量 4
        // 分配会使每个存活实例长期多占三份 lease 的空间。只为实际立即输入分配。
        let count = self
            .slots
            .iter()
            .filter(|slot| {
                matches!(
                    slot,
                    ConsumptionSlot::Available(ConstructionInput {
                        source: InputSource::Immediate { .. },
                        ..
                    })
                )
            })
            .count();
        let mut dependencies = Vec::with_capacity(count);
        for slot in &self.slots {
            match slot {
                ConsumptionSlot::Available(ConstructionInput {
                    source: InputSource::Immediate { lease, .. },
                    ..
                }) => dependencies.push(lease.clone()),
                ConsumptionSlot::Available(_) => {}
                ConsumptionSlot::Consumed => {
                    panic!("构造输入的保活集合必须在 adapter 消费前取得")
                }
            }
        }
        dependencies
    }

    /// 直接取得持有真实实例 lease 的必选令牌。
    pub fn take<T: Injectable + ?Sized>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Injection<T>, ConstructionError> {
        let input = self.available(slot)?;
        input.validate::<T>(slot, InputKind::Required)?;
        let token = input
            .project::<T>(slot)?
            .ok_or(ConstructionError::RequiredDependencyAbsent { slot })?;
        self.consume(slot);
        Ok(token)
    }

    /// 直接取得可选令牌；None 是准确的已交付缺席值。
    pub fn take_optional<T: Injectable + ?Sized>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<Injection<T>>, ConstructionError> {
        let input = self.available(slot)?;
        input.validate::<T>(slot, InputKind::Optional)?;
        let token = input.project::<T>(slot)?;
        self.consume(slot);
        Ok(token)
    }

    /// 只交付延迟句柄，不请求目标或收纳目标 lease。
    pub fn take_lazy<T: Injectable + ?Sized>(
        &mut self,
        slot: InputSlot,
    ) -> Result<LazyInjection<T>, ConstructionError> {
        self.available(slot)?
            .validate::<T>(slot, InputKind::LazyRequired)?;
        let InputSource::Lazy(dependency) = self.consume(slot).source else {
            unreachable!("必选延迟输入必须在创建时完成来源检查");
        };
        Ok(LazyInjection::new(dependency, slot))
    }

    /// 缺席来自冻结计划；未来目标的初始化错误不会变成 None。
    pub fn take_optional_lazy<T: Injectable + ?Sized>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<LazyInjection<T>>, ConstructionError> {
        self.available(slot)?
            .validate::<T>(slot, InputKind::LazyOptional)?;
        match self.consume(slot).source {
            InputSource::Absent => Ok(None),
            InputSource::Lazy(dependency) => Ok(Some(LazyInjection::new(dependency, slot))),
            InputSource::Immediate { .. } => {
                unreachable!("可选延迟输入必须在创建时完成来源检查")
            }
        }
    }

    /// 拒绝遗漏读取的输入，使用户构造只在全部参数验证成功后执行。
    pub fn ensure_all_consumed(&self) -> Result<(), ConstructionError> {
        let Some(index) = self
            .slots
            .iter()
            .position(|slot| matches!(slot, ConsumptionSlot::Available(_)))
        else {
            return Ok(());
        };
        Err(ConstructionError::UnconsumedSlot {
            slot: InputSlot::new(index),
        })
    }

    /// 取得尚未消费的槽位，分别报告越界和重复消费。
    fn available(&self, slot: InputSlot) -> Result<&ConstructionInput, ConstructionError> {
        let slot_count = self.slots.len();
        match self.slots.get(slot.index()) {
            Some(ConsumptionSlot::Available(input)) => Ok(input),
            Some(ConsumptionSlot::Consumed) => Err(ConstructionError::SlotAlreadyConsumed { slot }),
            None => Err(ConstructionError::SlotOutOfBounds { slot, slot_count }),
        }
    }

    /// 只在形态、类型及可能的投影均成功后执行。
    fn consume(&mut self, slot: InputSlot) -> ConstructionInput {
        let value = mem::replace(
            self.slots
                .get_mut(slot.index())
                .expect("槽位必须在消费前完成范围检查"),
            ConsumptionSlot::Consumed,
        );
        let ConsumptionSlot::Available(input) = value else {
            unreachable!("槽位必须在消费前确认尚未被消费");
        };
        input
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/inputs.rs"]
mod tests;
