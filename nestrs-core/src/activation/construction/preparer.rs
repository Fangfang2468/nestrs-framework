//! 将解析好的稳定服务地址转换为构造输入。
//!
//! concrete 输入先检查实例的准确类型，trait 输入还要执行由 rustc 检查过的类型化
//! 投影。两者最终都得到持有同一个真实实例 lease 的注入令牌。本模块不写槽位；只有
//! 完整成功后才返回 [`PreparedInput`]，由准备事务决定是否提交。

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

    // SAFETY: cast_input 已按准确 T 检查指针类型，返回的 lease 持有同一稳定实例。
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
            // SAFETY: cast_input 已按准确 T 检查指针类型，lease 持有同一稳定实例。
            unsafe { Injection::from_service_ptr(pointer, lease) }
        });
    Ok(PreparedInput::optional(token))
}

/// 为没有匹配 binding 的可选 trait 输入准备 `None`。
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

/// 用编译器生成并检查过的类型化投影准备必选 trait 输入。
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

    // SAFETY: 类型化投影的返回借用不长于 concrete 借用；lease 保活同一具体实例，
    // 指针包含真实投影产生的完整 trait 元数据，没有手工拼装 vtable。
    Ok(PreparedInput::required(unsafe {
        Injection::from_service_ptr(trait_pointer, lease)
    }))
}

/// 用编译器生成并检查过的类型化投影准备可选 trait 输入。
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
            // SAFETY: 类型化投影保留借用存活期，lease 保活同一 concrete 实例；
            // 仅包装真实投影产生的地址，不伪造 trait 元数据。
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
    // SAFETY: 所有调用点都先通过 cast_input 检查准确 concrete 类型，并在调用期间
    // 持有对应 lease。project 的高阶借用签名保证结果不会比输入借用活得更久。
    let concrete = unsafe { concrete.as_ref() };
    NonNull::from(project(concrete))
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/preparer.rs"]
mod tests;
