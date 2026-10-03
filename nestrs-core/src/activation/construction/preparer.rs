//! 将解析好的稳定服务地址转换为构造输入。
//!
//! concrete 输入先检查实例的准确类型，trait 输入还要执行由 rustc 检查过的类型化
//! 投影。两者最终都得到持有同一个真实实例 lease 的注入令牌。本模块不写槽位；普通
//! 输入完整准备后返回 [`PreparedInput`]，由准备事务决定是否提交。延迟输入先交付
//! 句柄，首次获取目标由独立的无装箱投影交付令牌，两条路径复用同一类型检查和投影。

use super::{
    ConstructionError, InputSlot, PreparedInput,
    lazy_dependency::LazyDependency,
    projection::{bound_token, required_token},
};
use crate::{
    activation::{LazyInjection, erased_service::ErasedServiceRef},
    service::Injectable,
};

/// 宏为一个字段或 factory 参数单态化生成的输入准备函数。
///
/// preparer 不直接改写共享 buffer。它必须先完整验证并返回一个 [`PreparedInput`]；
/// `ActivationPreparation` 只有在成功后才会把该值写入槽位并收纳对应 dependency lease。
pub type InputPreparer =
    fn(InputSlot, Option<ErasedServiceRef>) -> Result<PreparedInput, ConstructionError>;

/// 延迟字段的准备函数只交付句柄，不取得服务地址，也不触发目标构造。
pub type LazyInputPreparer =
    fn(InputSlot, Option<LazyDependency>) -> Result<PreparedInput, ConstructionError>;

/// 为必选延迟字段交付类型化句柄。
#[doc(hidden)]
pub fn prepare_lazy_required<T>(
    slot: InputSlot,
    input: Option<LazyDependency>,
) -> Result<PreparedInput, ConstructionError>
where
    T: Injectable + ?Sized,
{
    let dependency = input.ok_or(ConstructionError::RequiredDependencyAbsent { slot })?;
    Ok(PreparedInput::lazy_required(LazyInjection::<T>::new(
        dependency, slot,
    )))
}

/// 缺席可选依赖在构造时就是 `None`；已有候选才交付稍后可获取的句柄。
#[doc(hidden)]
pub fn prepare_lazy_optional<T>(
    slot: InputSlot,
    input: Option<LazyDependency>,
) -> Result<PreparedInput, ConstructionError>
where
    T: Injectable + ?Sized,
{
    Ok(PreparedInput::lazy_optional(input.map(|dependency| {
        LazyInjection::<T>::new(dependency, slot)
    })))
}

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
    Ok(PreparedInput::required(required_token::<T>(slot, input)?))
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
        .map(|input| required_token::<T>(slot, input))
        .transpose()?;
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
    Ok(PreparedInput::required(bound_token::<Concrete, Trait>(
        slot, input, project,
    )?))
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
        .map(|input| bound_token::<Concrete, Trait>(slot, input, project))
        .transpose()?;
    Ok(PreparedInput::optional(token))
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/preparer.rs"]
mod tests;
