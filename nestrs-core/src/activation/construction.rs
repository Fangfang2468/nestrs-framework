//! 宏生成构造 adapter 使用的隐藏 ABI。
//!
//! 本模块只描述已经绑定好的输入如何交给 class 或 factory adapter，不负责选择
//! provider、管理实例、调度 future 或暴露 resolve API。

use std::{any::Any, ptr::NonNull};

use thiserror::Error;

use crate::service::{Injectable, ServiceSource};

use super::{erased_service::ErasedServiceRef, injection::Injection};

/// 构造输入在 provider adapter 中的位置。
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputPosition(pub usize);

/// 宏生成构造 adapter 可能返回的受控错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActivationError {
    #[error("构造输入位置 {position:?} 不存在")]
    MissingInput { position: InputPosition },

    #[error("构造输入位置 {position:?} 已被消费")]
    InputAlreadyTaken { position: InputPosition },

    #[error("构造输入位置 {position:?} 已经设置")]
    InputAlreadyProvided { position: InputPosition },

    #[error("构造输入位置 {position:?} 超出可表示范围")]
    InputPositionOverflow { position: InputPosition },

    #[error("构造输入位置 {position:?} 需要必选依赖")]
    RequiredInputExpected { position: InputPosition },

    #[error("构造输入位置 {position:?} 需要可选依赖")]
    OptionalInputExpected { position: InputPosition },

    #[error("构造输入位置 {position:?} 的类型不匹配：期望 {expected}，实际为 {actual}")]
    InputTypeMismatch {
        position: InputPosition,
        expected: &'static str,
        actual: &'static str,
    },

    #[error("构造输入位置 {position:?} 的 trait 类型 {trait_type} 缺少 concrete-to-trait 投影")]
    UnprojectedTraitInput {
        position: InputPosition,
        trait_type: &'static str,
    },

    #[error("factory provider {provider}（{provider_source:?}）执行失败")]
    FactoryFailed {
        provider: &'static str,
        provider_source: ServiceSource,
    },
}

/// 由未来容器在 adapter 调用前准备的一个输入。
enum PreparedInput {
    Required {
        value: Box<dyn Any + Send + Sync>,
        service_type_name: &'static str,
    },
    Optional {
        value: Box<dyn Any + Send + Sync>,
        service_type_name: &'static str,
    },
}

/// 一个 provider 的已绑定构造输入。
///
/// 宏生成的 class adapter 按固定位置取走输入；此类型没有服务定位能力。
#[derive(Default)]
pub struct ConstructionContext {
    inputs: Vec<Option<PreparedInput>>,
}

impl ConstructionContext {
    /// 创建没有构造输入的上下文。
    pub fn new() -> Self {
        Self::default()
    }

    /// 将已验证的稳定地址包装成必选字段注入 token。
    ///
    /// # Safety
    ///
    /// pointer 必须指向当前容器拥有的精确 T，并且其有效期必须覆盖由该 adapter
    /// 构造出的消费者。
    pub(crate) unsafe fn insert_required_ptr<T>(
        &mut self,
        position: InputPosition,
        pointer: NonNull<T>,
    ) -> Result<(), ActivationError>
    where
        T: Injectable + ?Sized,
    {
        let value: Box<dyn Any + Send + Sync> =
            Box::new(unsafe { Injection::from_service_ptr(pointer) });

        self.insert(
            position,
            PreparedInput::Required {
                value,
                service_type_name: std::any::type_name::<T>(),
            },
        )
    }

    /// 将已验证的稳定地址包装成可选字段注入 token。
    ///
    /// # Safety
    ///
    /// present pointer 必须满足与 insert_required_ptr 相同的生命周期和类型前提。
    pub(crate) unsafe fn insert_optional_ptr<T>(
        &mut self,
        position: InputPosition,
        pointer: Option<NonNull<T>>,
    ) -> Result<(), ActivationError>
    where
        T: Injectable + ?Sized,
    {
        let token: Option<Injection<T>> =
            pointer.map(|pointer| unsafe { Injection::from_service_ptr(pointer) });
        let value: Box<dyn Any + Send + Sync> = Box::new(token);

        self.insert(
            position,
            PreparedInput::Optional {
                value,
                service_type_name: std::any::type_name::<T>(),
            },
        )
    }

    /// 取走一个必选字段注入 token。
    pub fn take<T>(&mut self, position: InputPosition) -> Result<Injection<T>, ActivationError>
    where
        T: Injectable + ?Sized,
    {
        match self.take_input(position)? {
            PreparedInput::Required {
                value,
                service_type_name,
            } => value
                .downcast::<Injection<T>>()
                .map(|value| *value)
                .map_err(|_| ActivationError::InputTypeMismatch {
                    position,
                    expected: std::any::type_name::<T>(),
                    actual: service_type_name,
                }),
            PreparedInput::Optional { .. } => {
                Err(ActivationError::RequiredInputExpected { position })
            }
        }
    }

    /// 取走一个可选注入 token。
    pub fn take_optional<T>(
        &mut self,
        position: InputPosition,
    ) -> Result<Option<Injection<T>>, ActivationError>
    where
        T: Injectable + ?Sized,
    {
        match self.take_input(position)? {
            PreparedInput::Optional {
                value,
                service_type_name,
            } => value
                .downcast::<Option<Injection<T>>>()
                .map(|value| *value)
                .map_err(|_| ActivationError::InputTypeMismatch {
                    position,
                    expected: std::any::type_name::<T>(),
                    actual: service_type_name,
                }),
            PreparedInput::Required { .. } => {
                Err(ActivationError::OptionalInputExpected { position })
            }
        }
    }

    fn insert(
        &mut self,
        position: InputPosition,
        input: PreparedInput,
    ) -> Result<(), ActivationError> {
        let required_len = position
            .0
            .checked_add(1)
            .ok_or(ActivationError::InputPositionOverflow { position })?;

        if self.inputs.len() < required_len {
            self.inputs.resize_with(required_len, || None);
        }

        let slot = self
            .inputs
            .get_mut(position.0)
            .expect("input vector was extended to include the requested position");

        if slot.is_some() {
            return Err(ActivationError::InputAlreadyProvided { position });
        }

        *slot = Some(input);
        Ok(())
    }

    fn take_input(&mut self, position: InputPosition) -> Result<PreparedInput, ActivationError> {
        let Some(slot) = self.inputs.get_mut(position.0) else {
            return Err(ActivationError::MissingInput { position });
        };

        slot.take()
            .ok_or(ActivationError::InputAlreadyTaken { position })
    }
}

/// factory 调用期间由 core 持有的私有 activation frame。
///
/// 未来 runtime 必须让此 frame 借用或持有所有 factory 输入服务的 owner lease。它不是
/// 单纯的生命周期标记：在 frame 结束前，所有已绑定输入都必须保持地址稳定且不可释放。
pub(crate) struct FactoryActivationFrame {
    _private: (),
}

/// 仅供 factory adapter 消费的 frame-bound 构造输入。
#[doc(hidden)]
pub struct FactoryConstructionContext<'frame> {
    bound: ConstructionContext,
    _frame: &'frame FactoryActivationFrame,
}

impl<'frame> FactoryConstructionContext<'frame> {
    /// 从已绑定输入与实际 activation frame 建立 factory context。
    ///
    /// # Safety
    ///
    /// 调用方必须保证 `frame` 在 `'frame` 内持有或借用每个 `bound` 输入所指向服务的
    /// owner lease。任何输入 owner 都不得在 factory future 完成前释放、迁移或替换；
    /// 否则 [`Self::take`] 签发的 `&'frame T` 会失效。
    #[allow(dead_code)] // activation runtime 接线后由其调用。
    pub(crate) unsafe fn from_bound(
        bound: ConstructionContext,
        frame: &'frame FactoryActivationFrame,
    ) -> Self {
        Self {
            bound,
            _frame: frame,
        }
    }

    /// 取走一个只在当前 factory 调用期间有效的必选服务借用。
    ///
    /// 与存入服务字段的 [`Injection`] 不同，factory 参数不获得可长期保存的 token。
    /// 返回值绑定到这个私有 activation frame，因此无法安全地放进 `'static` 输出或
    /// detached task。
    pub fn take<T>(&mut self, position: InputPosition) -> Result<&'frame T, ActivationError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.bound.take::<T>(position)?;

        // SAFETY: ConstructionContext only issues tokens from an already-validated stable
        // address, and from_bound's Safety contract keeps its owner alive for 'frame.
        Ok(unsafe { token.into_ptr().as_ref() })
    }

    /// 取走一个只在当前 factory 调用期间有效的可选服务借用。
    pub fn take_optional<T>(
        &mut self,
        position: InputPosition,
    ) -> Result<Option<&'frame T>, ActivationError>
    where
        T: Injectable + ?Sized,
    {
        let token = self.bound.take_optional::<T>(position)?;

        token
            .map(|token| {
                // SAFETY: see take; None contains no address.
                unsafe { Ok(token.into_ptr().as_ref()) }
            })
            .transpose()
    }
}

/// 宏为一个字段单态化生成的输入准备函数。
#[doc(hidden)]
pub type PrepareInput = fn(
    &mut ConstructionContext,
    InputPosition,
    Option<ErasedServiceRef>,
) -> Result<(), ActivationError>;

/// 将必选 concrete 输入写入构造上下文。
#[doc(hidden)]
pub fn prepare_required<T>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ErasedServiceRef>,
) -> Result<(), ActivationError>
where
    T: Injectable,
{
    let input = input.ok_or(ActivationError::MissingInput { position })?;
    let pointer = input.cast::<T>(position)?;

    // SAFETY: cast verified T and the future container must keep the source storage alive.
    unsafe { context.insert_required_ptr(position, pointer) }
}

/// 将可选 concrete 输入写入构造上下文。
#[doc(hidden)]
pub fn prepare_optional<T>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ErasedServiceRef>,
) -> Result<(), ActivationError>
where
    T: Injectable,
{
    let pointer = input.map(|input| input.cast::<T>(position)).transpose()?;

    // SAFETY: every present pointer passed type validation above.
    unsafe { context.insert_optional_ptr(position, pointer) }
}

/// 为没有 bind 的可选 trait 输入写入 None。
#[doc(hidden)]
pub fn prepare_optional_absent<T>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ErasedServiceRef>,
) -> Result<(), ActivationError>
where
    T: Injectable + ?Sized,
{
    if input.is_some() {
        return Err(ActivationError::UnprojectedTraitInput {
            position,
            trait_type: std::any::type_name::<T>(),
        });
    }

    // SAFETY: no pointer is stored for an absent optional trait input.
    unsafe { context.insert_optional_ptr::<T>(position, None) }
}

/// 用 bind 宏生成的 projector 准备必选 trait 输入。
#[doc(hidden)]
pub fn prepare_bound_required<Concrete, Trait>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ErasedServiceRef>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<(), ActivationError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    let input = input.ok_or(ActivationError::MissingInput { position })?;
    let concrete = input.cast::<Concrete>(position)?;
    let trait_pointer = project_bound_pointer(concrete, project);

    // SAFETY: the projector creates a valid trait-object pointer from the checked concrete ref.
    unsafe { context.insert_required_ptr(position, trait_pointer) }
}

/// 用 bind 宏生成的 projector 准备可选 trait 输入。
#[doc(hidden)]
pub fn prepare_bound_optional<Concrete, Trait>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ErasedServiceRef>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<(), ActivationError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    let trait_pointer = input
        .map(|input| input.cast::<Concrete>(position))
        .transpose()
        .map(|concrete| concrete.map(|concrete| project_bound_pointer(concrete, project)))?;

    // SAFETY: present pointers originate from the typed projector; absent inputs carry no ptr.
    unsafe { context.insert_optional_ptr(position, trait_pointer) }
}

fn project_bound_pointer<Concrete, Trait>(
    concrete: NonNull<Concrete>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> NonNull<Trait>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    // SAFETY: the concrete pointer was checked against Concrete by ErasedServiceRef::cast.
    let concrete = unsafe { concrete.as_ref() };
    NonNull::from(project(concrete))
}
