//! 将解析好的稳定服务地址转换为构造输入。
//!
//! concrete 输入先检查实例的准确类型，trait 输入还要执行由 rustc 检查过的类型化
//! 投影。两者最终都得到持有同一个真实实例 lease 的注入令牌。本模块不写槽位；普通
//! [`InputPreparation`] 与 [`LazyInputPreparation`] 分别拥有一次普通/延迟输入准备。
//! 它们消费输入并返回 [`PreparedInput`]，由准备事务决定是否提交。文件末尾的
//! `prepare_*` 只保留编译器回调协议；类型检查与投影集中在 [`ServiceProjection`]。

use super::{
    ConstructionError, InputSlot, PreparedInput, lazy_dependency::LazyDependency,
    projection::ServiceProjection,
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

/// 一个普通依赖输入的准备过程；只拥有当前槽位和该输入的实例凭证。
///
/// 消费 self 后才交付完整载荷，不保存 buffer、计划或运行时状态。required 与
/// optional 共享同一个 ServiceProjection 类型恢复边界，输入缺席仍保留准确形态。
struct InputPreparation {
    slot: InputSlot,
    input: Option<ErasedServiceRef>,
}

impl InputPreparation {
    fn new(slot: InputSlot, input: Option<ErasedServiceRef>) -> Self {
        Self { slot, input }
    }

    fn required<T: Injectable + ?Sized>(self) -> Result<PreparedInput, ConstructionError> {
        Ok(PreparedInput::required(
            self.required_source()?.concrete::<T>()?,
        ))
    }

    fn optional<T: Injectable + ?Sized>(self) -> Result<PreparedInput, ConstructionError> {
        let token = self
            .input
            .map(|input| ServiceProjection::new(self.slot, input).concrete::<T>())
            .transpose()?;
        Ok(PreparedInput::optional(token))
    }

    fn optional_absent<T: Injectable + ?Sized>(self) -> Result<PreparedInput, ConstructionError> {
        if self.input.is_some() {
            return Err(ConstructionError::UnprojectedTraitInput {
                slot: self.slot,
                trait_type: std::any::type_name::<T>(),
            });
        }
        Ok(PreparedInput::optional::<T>(None))
    }

    fn bound_required<Concrete, Trait>(
        self,
        project: for<'a> fn(&'a Concrete) -> &'a Trait,
    ) -> Result<PreparedInput, ConstructionError>
    where
        Concrete: Injectable,
        Trait: Injectable + ?Sized,
    {
        Ok(PreparedInput::required(
            self.required_source()?.bound(project)?,
        ))
    }

    fn bound_optional<Concrete, Trait>(
        self,
        project: for<'a> fn(&'a Concrete) -> &'a Trait,
    ) -> Result<PreparedInput, ConstructionError>
    where
        Concrete: Injectable,
        Trait: Injectable + ?Sized,
    {
        let token = self
            .input
            .map(|input| ServiceProjection::new(self.slot, input).bound(project))
            .transpose()?;
        Ok(PreparedInput::optional(token))
    }

    fn required_source(self) -> Result<ServiceProjection, ConstructionError> {
        let input = self
            .input
            .ok_or(ConstructionError::RequiredDependencyAbsent { slot: self.slot })?;
        Ok(ServiceProjection::new(self.slot, input))
    }
}

/// 一个延迟输入的准备过程；拥有尚未发起请求的描述与弱 owner 能力。
/// 它只创建句柄，不能执行目标构造，也不为尚不存在的实例创建 lease。
struct LazyInputPreparation {
    slot: InputSlot,
    input: Option<LazyDependency>,
}

impl LazyInputPreparation {
    fn new(slot: InputSlot, input: Option<LazyDependency>) -> Self {
        Self { slot, input }
    }

    fn required<T: Injectable + ?Sized>(self) -> Result<PreparedInput, ConstructionError> {
        let dependency = self
            .input
            .ok_or(ConstructionError::RequiredDependencyAbsent { slot: self.slot })?;
        Ok(PreparedInput::lazy_required(LazyInjection::<T>::new(
            dependency, self.slot,
        )))
    }

    fn optional<T: Injectable + ?Sized>(self) -> PreparedInput {
        PreparedInput::lazy_optional(
            self.input
                .map(|dependency| LazyInjection::<T>::new(dependency, self.slot)),
        )
    }
}

// 以下是编译器生成 adapter 使用的函数指针入口。保留真实路径和签名，
// 普通实现只阅读上面的两个输入对象；此边界不持有状态，也不重复业务逻辑。

/// 为必选延迟字段交付类型化句柄。
#[doc(hidden)]
pub fn prepare_lazy_required<T>(
    slot: InputSlot,
    input: Option<LazyDependency>,
) -> Result<PreparedInput, ConstructionError>
where
    T: Injectable + ?Sized,
{
    LazyInputPreparation::new(slot, input).required::<T>()
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
    Ok(LazyInputPreparation::new(slot, input).optional::<T>())
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
    InputPreparation::new(slot, input).required::<T>()
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
    InputPreparation::new(slot, input).optional::<T>()
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
    InputPreparation::new(slot, input).optional_absent::<T>()
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
    InputPreparation::new(slot, input).bound_required(project)
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
    InputPreparation::new(slot, input).bound_optional(project)
}

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/preparer.rs"]
mod tests;
