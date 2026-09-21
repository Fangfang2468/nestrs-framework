//! 固定槽位的构造输入状态机。

use std::{any::Any, mem, ptr::NonNull};

use crate::{activation::Injection, service::Injectable};

use super::{error::ConstructionError, slot::InputSlot};

/// 已经完成类型化准备、但尚未写入固定槽位的一个输入。
///
/// 未来 `InputPreparer` 只会产生此值；实际写入 buffer 与收纳 dependency lease 的动作
/// 由 activation runtime 集中完成，因此 preparer 无法留下半写入状态。
#[doc(hidden)]
pub struct PreparedInput {
    kind: PreparedInputKind,
}

enum PreparedInputKind {
    Required {
        value: Box<dyn Any + Send + Sync>,
        service_type_name: &'static str,
    },
    Optional {
        value: Box<dyn Any + Send + Sync>,
        service_type_name: &'static str,
    },
}

impl PreparedInput {
    /// 将已经验证过的稳定地址包装成必选字段注入 token。
    ///
    /// # Safety
    ///
    /// `pointer` 必须指向精确的 `T`，并且实例 owner 必须在所有消费该 token 的对象
    /// 销毁前保持地址有效。
    pub(in crate::activation::construction) unsafe fn required<T>(pointer: NonNull<T>) -> Self
    where
        T: Injectable + ?Sized,
    {
        Self {
            kind: PreparedInputKind::Required {
                value: Box::new(unsafe { Injection::from_service_ptr(pointer) }),
                service_type_name: std::any::type_name::<T>(),
            },
        }
    }

    /// 将已经验证过的稳定地址包装成可选字段注入 token。
    ///
    /// # Safety
    ///
    /// `Some(pointer)` 的安全前提与 [`Self::required`] 相同；`None` 表示一个已经准备
    /// 完成的可选缺席输入，而不是未填充的槽位。
    pub(in crate::activation::construction) unsafe fn optional<T>(
        pointer: Option<NonNull<T>>,
    ) -> Self
    where
        T: Injectable + ?Sized,
    {
        let token = pointer.map(|pointer| unsafe { Injection::from_service_ptr(pointer) });

        Self {
            kind: PreparedInputKind::Optional {
                value: Box::new(token),
                service_type_name: std::any::type_name::<T>(),
            },
        }
    }
}

/// runtime 在调用 adapter 前使用的固定长度输入写入器。
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

            prepared.push(ConsumptionSlot::Available(input));
        }

        Ok(ConstructionInputs { slots: prepared })
    }
}

/// 已完成绑定、仅供 class adapter 消费的构造输入。
///
/// 该类型不提供写入 API；adapter 只能按 slot 取得匹配的 required 或 optional token，
/// 并在构造完成前通过 [`Self::ensure_all_consumed`] 验证 descriptor 与 adapter 一致。
#[doc(hidden)]
pub struct ConstructionInputs {
    slots: Vec<ConsumptionSlot>,
}

enum ConsumptionSlot {
    Available(PreparedInput),
    Consumed,
}

impl ConstructionInputs {
    /// 创建没有依赖的合法构造输入，供零依赖 adapter 使用。
    pub fn empty() -> Self {
        InputBuffer::new(0)
            .finish()
            .expect("an empty fixed input buffer is complete")
    }

    /// 取走一个必选字段注入 token。
    pub fn take<T>(&mut self, slot: InputSlot) -> Result<Injection<T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.ensure_required_type::<T>(slot)?;

        let PreparedInput {
            kind:
                PreparedInputKind::Required {
                    value,
                    service_type_name: _,
                },
        } = self.take_available(slot)
        else {
            unreachable!("required input was checked before consumption");
        };

        Ok(*value
            .downcast::<Injection<T>>()
            .expect("required input type was checked before consumption"))
    }

    /// 取走一个可选字段注入 token。
    pub fn take_optional<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<Injection<T>>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        self.ensure_optional_type::<T>(slot)?;

        let PreparedInput {
            kind:
                PreparedInputKind::Optional {
                    value,
                    service_type_name: _,
                },
        } = self.take_available(slot)
        else {
            unreachable!("optional input was checked before consumption");
        };

        Ok(*value
            .downcast::<Option<Injection<T>>>()
            .expect("optional input type was checked before consumption"))
    }

    /// 拒绝仍遗留在 adapter 输入中的槽位。
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

    fn ensure_required_type<T>(&self, slot: InputSlot) -> Result<(), ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let input = self.available(slot)?;
        match &input.kind {
            PreparedInputKind::Required {
                value,
                service_type_name,
            } => {
                if value.is::<Injection<T>>() {
                    Ok(())
                } else {
                    Err(ConstructionError::InputTypeMismatch {
                        slot,
                        expected: std::any::type_name::<T>(),
                        actual: service_type_name,
                    })
                }
            }
            PreparedInputKind::Optional { .. } => {
                Err(ConstructionError::RequiredInputExpected { slot })
            }
        }
    }

    fn ensure_optional_type<T>(&self, slot: InputSlot) -> Result<(), ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let input = self.available(slot)?;
        match &input.kind {
            PreparedInputKind::Optional {
                value,
                service_type_name,
            } => {
                if value.is::<Option<Injection<T>>>() {
                    Ok(())
                } else {
                    Err(ConstructionError::InputTypeMismatch {
                        slot,
                        expected: std::any::type_name::<T>(),
                        actual: service_type_name,
                    })
                }
            }
            PreparedInputKind::Required { .. } => {
                Err(ConstructionError::OptionalInputExpected { slot })
            }
        }
    }

    fn available(&self, slot: InputSlot) -> Result<&PreparedInput, ConstructionError> {
        let slot_count = self.slots.len();
        let Some(value) = self.slots.get(slot.index()) else {
            return Err(ConstructionError::SlotOutOfBounds { slot, slot_count });
        };

        match value {
            ConsumptionSlot::Available(input) => Ok(input),
            ConsumptionSlot::Consumed => Err(ConstructionError::SlotAlreadyConsumed { slot }),
        }
    }

    fn take_available(&mut self, slot: InputSlot) -> PreparedInput {
        let value = mem::replace(
            self.slots
                .get_mut(slot.index())
                .expect("slot was checked before consumption"),
            ConsumptionSlot::Consumed,
        );

        let ConsumptionSlot::Available(input) = value else {
            unreachable!("slot was checked before consumption");
        };

        input
    }
}

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use super::super::slot::InputSlot;
    use super::{ConstructionInputs, InputBuffer, PreparedInput};
    use crate::{activation::Injection, activation::construction::error::ConstructionError};

    #[derive(Debug)]
    struct Alpha;

    #[derive(Debug)]
    struct Beta;

    #[derive(Debug)]
    struct Tagged(u8);

    fn required<T>(value: &mut T) -> PreparedInput
    where
        T: Send + Sync + 'static,
    {
        // SAFETY: test locals outlive the input token and all assertions that dereference it.
        unsafe { PreparedInput::required(NonNull::from(value)) }
    }

    fn optional<T>(value: Option<&mut T>) -> PreparedInput
    where
        T: Send + Sync + 'static,
    {
        // SAFETY: test locals outlive the input token and all assertions that dereference it.
        unsafe { PreparedInput::optional(value.map(NonNull::from)) }
    }

    #[test]
    fn accepts_out_of_order_preparation_and_consumes_each_slot_once() {
        let mut alpha = Alpha;
        let mut beta = Beta;
        let mut buffer = InputBuffer::new(2);

        buffer
            .insert(InputSlot::new(1), optional(Some(&mut beta)))
            .expect("second slot should accept the first preparation");
        buffer
            .insert(InputSlot::new(0), required(&mut alpha))
            .expect("first slot should accept a later preparation");

        let mut inputs = buffer.finish().expect("all slots are ready");
        let _: Injection<Alpha> = inputs
            .take(InputSlot::new(0))
            .expect("required slot should yield Alpha");
        let optional_beta: Option<Injection<Beta>> = inputs
            .take_optional(InputSlot::new(1))
            .expect("optional slot should yield Beta");
        assert!(optional_beta.is_some());
        inputs
            .ensure_all_consumed()
            .expect("all prepared slots should be consumed");
    }

    #[test]
    fn rejects_an_out_of_bounds_slot_without_changing_the_buffer() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(1);

        assert_eq!(
            buffer.insert(InputSlot::new(1), required(&mut alpha)),
            Err(ConstructionError::SlotOutOfBounds {
                slot: InputSlot::new(1),
                slot_count: 1,
            })
        );
        assert!(matches!(
            buffer.finish(),
            Err(ConstructionError::UnfilledSlot { slot }) if slot == InputSlot::new(0)
        ));
    }

    #[test]
    fn rejects_duplicate_preparation() {
        let mut first = Tagged(1);
        let mut second = Tagged(2);
        let mut buffer = InputBuffer::new(1);

        buffer
            .insert(InputSlot::new(0), required(&mut first))
            .expect("first preparation should succeed");
        assert_eq!(
            buffer.insert(InputSlot::new(0), required(&mut second)),
            Err(ConstructionError::SlotAlreadyPrepared {
                slot: InputSlot::new(0),
            })
        );
        let mut inputs = buffer.finish().expect("first prepared value must remain");
        let token = inputs
            .take::<Tagged>(InputSlot::new(0))
            .expect("first prepared value should remain");
        assert_eq!(token.0, 1);
    }

    #[test]
    fn finish_rejects_the_first_unfilled_slot() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(2);
        buffer
            .insert(InputSlot::new(0), required(&mut alpha))
            .expect("first slot should be ready");

        assert!(matches!(
            buffer.finish(),
            Err(ConstructionError::UnfilledSlot { slot }) if slot == InputSlot::new(1)
        ));
    }

    #[test]
    fn required_optional_mismatch_does_not_consume_the_slot() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(1);
        buffer
            .insert(InputSlot::new(0), optional(Some(&mut alpha)))
            .expect("optional slot should prepare");

        let mut inputs = buffer.finish().expect("slot should be complete");
        assert!(matches!(
            inputs.take::<Alpha>(InputSlot::new(0)),
            Err(ConstructionError::RequiredInputExpected { slot }) if slot == InputSlot::new(0)
        ));
        assert!(matches!(
            inputs.take_optional::<Alpha>(InputSlot::new(0)),
            Ok(Some(_))
        ));
    }

    #[test]
    fn optional_required_mismatch_does_not_consume_the_slot() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(1);
        buffer
            .insert(InputSlot::new(0), required(&mut alpha))
            .expect("required slot should prepare");

        let mut inputs = buffer.finish().expect("slot should be complete");
        assert!(matches!(
            inputs.take_optional::<Alpha>(InputSlot::new(0)),
            Err(ConstructionError::OptionalInputExpected { slot }) if slot == InputSlot::new(0)
        ));
        assert!(matches!(inputs.take::<Alpha>(InputSlot::new(0)), Ok(_)));
    }

    #[test]
    fn an_absent_optional_input_is_ready_not_unfilled() {
        let mut buffer = InputBuffer::new(1);
        buffer
            .insert(InputSlot::new(0), optional::<Alpha>(None))
            .expect("an absent optional dependency still prepares its slot");

        let mut inputs = buffer.finish().expect("absent optional slot is complete");
        assert!(matches!(
            inputs.take_optional::<Alpha>(InputSlot::new(0)),
            Ok(None)
        ));
        inputs
            .ensure_all_consumed()
            .expect("the optional slot should be consumed");
    }

    #[test]
    fn type_mismatch_does_not_consume_the_slot() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(1);
        buffer
            .insert(InputSlot::new(0), required(&mut alpha))
            .expect("required slot should prepare");

        let mut inputs = buffer.finish().expect("slot should be complete");
        assert!(matches!(
            inputs.take::<Beta>(InputSlot::new(0)),
            Err(ConstructionError::InputTypeMismatch {
                slot,
                expected,
                actual,
            }) if slot == InputSlot::new(0)
                && expected == std::any::type_name::<Beta>()
                && actual == std::any::type_name::<Alpha>()
        ));
        assert!(matches!(inputs.take::<Alpha>(InputSlot::new(0)), Ok(_)));
    }

    #[test]
    fn optional_type_mismatch_does_not_consume_the_slot() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(1);
        buffer
            .insert(InputSlot::new(0), optional(Some(&mut alpha)))
            .expect("optional slot should prepare");

        let mut inputs = buffer.finish().expect("slot should be complete");
        assert!(matches!(
            inputs.take_optional::<Beta>(InputSlot::new(0)),
            Err(ConstructionError::InputTypeMismatch {
                slot,
                expected,
                actual,
            }) if slot == InputSlot::new(0)
                && expected == std::any::type_name::<Beta>()
                && actual == std::any::type_name::<Alpha>()
        ));
        assert!(matches!(
            inputs.take_optional::<Alpha>(InputSlot::new(0)),
            Ok(Some(_))
        ));
    }

    #[test]
    fn rejects_second_consumption_of_a_slot() {
        let mut alpha = Alpha;
        let mut buffer = InputBuffer::new(1);
        buffer
            .insert(InputSlot::new(0), required(&mut alpha))
            .expect("required slot should prepare");

        let mut inputs = buffer.finish().expect("slot should be complete");
        let _: Injection<Alpha> = inputs
            .take(InputSlot::new(0))
            .expect("first consumption should succeed");
        assert!(matches!(
            inputs.take::<Alpha>(InputSlot::new(0)),
            Err(ConstructionError::SlotAlreadyConsumed { slot }) if slot == InputSlot::new(0)
        ));
    }

    #[test]
    fn reports_the_first_unconsumed_slot() {
        let mut alpha = Alpha;
        let mut beta = Beta;
        let mut buffer = InputBuffer::new(2);
        buffer
            .insert(InputSlot::new(0), required(&mut alpha))
            .expect("first slot should prepare");
        buffer
            .insert(InputSlot::new(1), required(&mut beta))
            .expect("second slot should prepare");

        let mut inputs = buffer.finish().expect("all slots should be complete");
        let _: Injection<Alpha> = inputs
            .take(InputSlot::new(0))
            .expect("first slot should consume");
        assert_eq!(
            inputs.ensure_all_consumed(),
            Err(ConstructionError::UnconsumedSlot {
                slot: InputSlot::new(1),
            })
        );
        let _: Injection<Beta> = inputs
            .take(InputSlot::new(1))
            .expect("the remaining slot should still be consumable");
        inputs
            .ensure_all_consumed()
            .expect("the failed check must not consume a slot");
    }

    #[test]
    fn empty_inputs_are_already_fully_consumed() {
        let mut inputs = ConstructionInputs::empty();
        inputs
            .ensure_all_consumed()
            .expect("empty input set should be valid");
        assert!(matches!(
            inputs.take::<Alpha>(InputSlot::new(0)),
            Err(ConstructionError::SlotOutOfBounds {
                slot,
                slot_count: 0,
            }) if slot == InputSlot::new(0)
        ));
    }
}
