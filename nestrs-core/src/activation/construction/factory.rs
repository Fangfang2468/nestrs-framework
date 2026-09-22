//! Factory provider 构造 adapter 的 ABI。

use std::{future::Future, marker::PhantomData, pin::Pin};

use super::{ConstructionError, ConstructionInputs, InputSlot};
use crate::{activation::erased_service::ErasedService, service::Injectable};

/// 异步 factory adapter 返回的 frame-bound future。
pub type FactoryFuture<'frame> =
    Pin<Box<dyn Future<Output = Result<ErasedService, ConstructionError>> + Send + 'frame>>;

/// 同步 factory adapter 的单态化签名。
pub type FactoryConstructor =
    for<'frame> fn(FactoryInputs<'frame>) -> Result<ErasedService, ConstructionError>;

/// 异步 factory adapter 的单态化签名。
pub type AsyncConstructor = for<'frame> fn(FactoryInputs<'frame>) -> FactoryFuture<'frame>;

/// 仅供 factory adapter 消费的 frame-bound 构造输入。
///
/// 该类型没有公开构造函数。下一单元的 `FactoryLeaseFrame` 将在本模块内创建它，并实际
/// 持有每个输入服务的 owner lease；在那之前 runtime 无法安全地调用 factory adapter。
/// 这避免了旧 `FactoryActivationFrame` 那种仅以 ZST 承诺服务存活期的设计。
#[doc(hidden)]
pub struct FactoryInputs<'frame> {
    inputs: ConstructionInputs,
    _lease_frame: PhantomData<&'frame ()>,
}

impl<'frame> FactoryInputs<'frame> {
    /// 取走一个只在本次 factory 调用期间有效的必选服务借用。
    pub fn take<T>(&mut self, slot: InputSlot) -> Result<&'frame T, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.inputs.take::<T>(slot)?;

        // SAFETY: FactoryInputs can only be constructed by the future FactoryLeaseFrame, which
        // keeps every dependency lease alive for 'frame.
        Ok(unsafe { token.into_ptr().as_ref() })
    }

    /// 取走一个只在本次 factory 调用期间有效的可选服务借用。
    pub fn take_optional<T>(
        &mut self,
        slot: InputSlot,
    ) -> Result<Option<&'frame T>, ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.inputs.take_optional::<T>(slot)?;

        Ok(token.map(|token| {
            // SAFETY: see `Self::take`; absent inputs contain no address.
            unsafe { token.into_ptr().as_ref() }
        }))
    }

    /// 拒绝 factory adapter 未消费的 descriptor 槽位。
    pub fn ensure_all_consumed(&self) -> Result<(), ConstructionError> {
        self.inputs.ensure_all_consumed()
    }
}
