//! 将解析好的稳定服务地址转换为构造输入。

use std::ptr::NonNull;

use super::{ConstructionError, InputSlot, PreparedInput};
use crate::{
    activation::{DependencyLease, Injection, erased_service::ErasedServiceRef},
    service::Injectable,
};

/// 宏为一个字段或 factory 参数单态化生成的输入准备函数。
///
/// preparer 不直接改写共享 buffer。它必须先完整验证并返回一个 [`PreparedInput`]；
/// `ActivationPreparation` 只有在成功后才会把该值写入槽位并收纳对应 dependency lease。
pub type InputPreparer =
    fn(InputSlot, Option<ErasedServiceRef>) -> Result<PreparedInput, ConstructionError>;

/// 将必选 concrete 输入准备为不可变载荷。
#[doc(hidden)]
pub fn prepare_required<T>(
    slot: InputSlot,
    input: Option<ErasedServiceRef>,
) -> Result<PreparedInput, ConstructionError>
where
    T: Injectable + ?Sized,
{
    let input = input.ok_or(ConstructionError::RequiredDependencyAbsent { slot })?;
    let (pointer, lease) = cast_input::<T>(slot, input)?;

    // SAFETY: exact type was checked and lease retains the allocation.
    Ok(PreparedInput::required(unsafe {
        Injection::from_service_ptr(pointer, lease)
    }))
}

/// 将可选 concrete 输入准备为不可变载荷。
#[doc(hidden)]
pub fn prepare_optional<T>(
    slot: InputSlot,
    input: Option<ErasedServiceRef>,
) -> Result<PreparedInput, ConstructionError>
where
    T: Injectable + ?Sized,
{
    let token = input
        .map(|input| cast_input::<T>(slot, input))
        .transpose()?
        .map(|(pointer, lease)| {
            // SAFETY: exact type was checked and lease retains the allocation.
            unsafe { Injection::from_service_ptr(pointer, lease) }
        });
    Ok(PreparedInput::optional(token))
}

/// 为没有 bind 的可选 trait 输入准备 `None`。
#[doc(hidden)]
pub fn prepare_optional_absent<T>(
    slot: InputSlot,
    input: Option<ErasedServiceRef>,
) -> Result<PreparedInput, ConstructionError>
where
    T: Injectable + ?Sized,
{
    if input.is_some() {
        return Err(ConstructionError::UnprojectedTraitInput {
            slot,
            trait_type: std::any::type_name::<T>(),
        });
    }

    Ok(PreparedInput::optional::<T>(None))
}

/// 用 bind 宏生成的 projector 准备必选 trait 输入。
#[doc(hidden)]
pub fn prepare_bound_required<Concrete, Trait>(
    slot: InputSlot,
    input: Option<ErasedServiceRef>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<PreparedInput, ConstructionError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    let input = input.ok_or(ConstructionError::RequiredDependencyAbsent { slot })?;
    let (concrete, lease) = cast_input::<Concrete>(slot, input)?;
    let trait_pointer = project_bound_pointer(concrete, project);

    // SAFETY: typed projection preserves the reference lifetime; lease retains concrete.
    Ok(PreparedInput::required(unsafe {
        Injection::from_service_ptr(trait_pointer, lease)
    }))
}

/// 用 bind 宏生成的 projector 准备可选 trait 输入。
#[doc(hidden)]
pub fn prepare_bound_optional<Concrete, Trait>(
    slot: InputSlot,
    input: Option<ErasedServiceRef>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<PreparedInput, ConstructionError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    let token = input
        .map(|input| cast_input::<Concrete>(slot, input))
        .transpose()?
        .map(|(concrete, lease)| {
            let pointer = project_bound_pointer(concrete, project);
            // SAFETY: typed projection preserves the reference lifetime; lease retains concrete.
            unsafe { Injection::from_service_ptr(pointer, lease) }
        });
    Ok(PreparedInput::optional(token))
}

fn cast_input<T>(
    slot: InputSlot,
    input: ErasedServiceRef,
) -> Result<(NonNull<T>, DependencyLease), ConstructionError>
where
    T: Injectable + ?Sized,
{
    input
        .cast::<T>()
        .map_err(|actual| ConstructionError::InputTypeMismatch {
            slot,
            expected: std::any::type_name::<T>(),
            actual: actual.name,
        })
}

fn project_bound_pointer<Concrete, Trait>(
    concrete: NonNull<Concrete>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> NonNull<Trait>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    // SAFETY: `cast_input` checked the concrete pointer before this projector runs.
    let concrete = unsafe { concrete.as_ref() };
    NonNull::from(project(concrete))
}

#[cfg(test)]
mod tests {
    use super::{
        prepare_bound_required, prepare_optional, prepare_optional_absent, prepare_required,
    };
    use crate::activation::{
        ConstructionError, DependencyLease, ErasedService, ErasedServiceRef, InputSlot,
        ReleaseDomain,
    };

    struct Alpha;
    struct Beta;

    fn erased<T>(value: T) -> ErasedServiceRef
    where
        T: Send + Sync + 'static,
    {
        DependencyLease::new(ErasedService::new(value), vec![], ReleaseDomain::new()).erased_ref()
    }

    #[test]
    fn required_preparer_reports_a_missing_dependency_without_a_buffer() {
        assert!(matches!(
            prepare_required::<Alpha>(InputSlot::new(0), None),
            Err(ConstructionError::RequiredDependencyAbsent { slot }) if slot == InputSlot::new(0)
        ));
    }

    #[test]
    fn preparer_maps_erased_type_mismatch_to_the_request_slot() {
        assert!(matches!(
            prepare_optional::<Beta>(InputSlot::new(3), Some(erased(Alpha))),
            Err(ConstructionError::InputTypeMismatch { slot, expected, actual })
                if slot == InputSlot::new(3)
                    && expected == std::any::type_name::<Beta>()
                    && actual == std::any::type_name::<Alpha>()
        ));
    }

    #[test]
    fn bound_projection_retains_its_owner_and_the_exact_trait_vtable() {
        trait Port: Send + Sync {
            fn value(&self) -> u32;
        }
        struct Adapter(u32);
        impl Port for Adapter {
            fn value(&self) -> u32 {
                self.0
            }
        }
        let input = erased(Adapter(73));
        let prepared = prepare_bound_required::<Adapter, dyn Port>(
            InputSlot::new(0),
            Some(input),
            |adapter| adapter,
        )
        .unwrap();
        let token = prepared
            .into_required::<dyn Port>(InputSlot::new(0))
            .unwrap();
        assert_eq!(token.value(), 73);
        assert!(matches!(
            prepare_optional_absent::<dyn Port>(InputSlot::new(2), Some(erased(Adapter(1)))),
            Err(ConstructionError::UnprojectedTraitInput { slot, .. }) if slot == InputSlot::new(2)
        ));
    }
}
