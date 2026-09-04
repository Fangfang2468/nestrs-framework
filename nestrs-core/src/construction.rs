//! 宏生成构造 adapter 使用的隐藏 activation ABI。
//!
//! [`ConstructionContext`] 只保存 compile 阶段已经绑定到固定位置的输入，不提供
//! `resolve` 一类的 service-locator API。`#[injectable]` 与未来的同步 `#[factory]`
//! adapter 都通过 [`Constructor`] 消费同一套 ABI。

use std::{alloc::Layout, any::Any, ptr::NonNull};

use thiserror::Error;

use crate::{
    arena::ArenaServiceRef,
    inject_wrapper::Inject,
    registration::{injectable::Injectable, service_type::ServiceType},
};

/// 构造输入在已编译 provider 中的位置。
///
/// 宏分析阶段已经使用 `usize` 为 `#[inject]` 字段编号，因此这里保持同一表示，避免
/// 引入额外的上限或隐式截断。
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputPosition(pub usize);

/// 构造 adapter 消费预绑定输入时可能发生的受控错误。
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
}

/// 由 runtime 在 adapter 调用前准备的一个输入。
///
/// `Inject<T>` 本身是 Sized，即使 `T` 是 `dyn Trait`，因此可以保存在 `Any` 擦除
/// 容器中，并在宏生成的 `take::<T>` 调用时恢复为准确的 token 类型。
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
/// 此类型按值传给 [`Constructor`]，从而每个输入槽只能被一个字段/参数消费一次。
/// 运行时负责根据已编译依赖图填充它；`insert_*` 也保留在隐藏 ABI 中，以供 core
/// 测试及将来的激活器构造输入。
#[derive(Default)]
pub struct ConstructionContext {
    inputs: Vec<Option<PreparedInput>>,
}

impl ConstructionContext {
    /// 创建没有任何构造输入的上下文。
    pub fn new() -> Self {
        Self::default()
    }

    /// 将 Arena 中已验证的必选服务地址封装为字段令牌。
    ///
    /// 该入口只能由同 crate 的 [`prepare_required`] 调用。传入地址必须指向当前
    /// Arena 中已提交且精确为 `T` 的服务，并且生成的 context 必须立刻交给同一次
    /// 宏构造 adapter；这样 `Inject<T>` 不会逃离拥有该地址的 Arena。
    pub(crate) unsafe fn insert_required_ptr<T>(
        &mut self,
        position: InputPosition,
        pointer: NonNull<T>,
    ) -> Result<(), ActivationError>
    where
        T: Injectable + ?Sized,
    {
        let value: Box<dyn Any + Send + Sync> =
            Box::new(unsafe { Inject::from_field_ptr(pointer) });

        self.insert(
            position,
            PreparedInput::Required {
                value,
                service_type_name: std::any::type_name::<T>(),
            },
        )
    }

    /// 将 Arena 中已验证的可选服务地址封装为字段令牌。
    ///
    /// `None` 仍会保留 `T` 的类型信息，因此宏生成的 `take_optional::<T>` 可以验证
    /// 自己消费的是正确的输入槽。
    pub(crate) unsafe fn insert_optional_ptr<T>(
        &mut self,
        position: InputPosition,
        pointer: Option<NonNull<T>>,
    ) -> Result<(), ActivationError>
    where
        T: Injectable + ?Sized,
    {
        let token: Option<Inject<T>> =
            pointer.map(|pointer| unsafe { Inject::from_field_ptr(pointer) });
        let value: Box<dyn Any + Send + Sync> = Box::new(token);

        self.insert(
            position,
            PreparedInput::Optional {
                value,
                service_type_name: std::any::type_name::<T>(),
            },
        )
    }

    /// 取走一个必选 `Inject<T>` 输入。
    ///
    /// 宏生成的构造 adapter 用它取得固定位置的必选输入。
    ///
    /// slot 的存在性、可选性与类型均会在这里校验；位置错误只会返回
    /// [`ActivationError`]，不会要求调用方维持额外的内存安全前提。
    pub fn take<T>(&mut self, position: InputPosition) -> Result<Inject<T>, ActivationError>
    where
        T: Injectable + ?Sized,
    {
        match self.take_input(position)? {
            PreparedInput::Required {
                value,
                service_type_name,
            } => value
                .downcast::<Inject<T>>()
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

    /// 取走一个可选 `Inject<T>` 输入。
    ///
    /// 宏生成的构造 adapter 用它取得固定位置的可选输入。
    ///
    /// 与 [`Self::take`] 一样，slot 的存在性、可选性与类型均由此方法验证。
    pub fn take_optional<T>(
        &mut self,
        position: InputPosition,
    ) -> Result<Option<Inject<T>>, ActivationError>
    where
        T: Injectable + ?Sized,
    {
        match self.take_input(position)? {
            PreparedInput::Optional {
                value,
                service_type_name,
            } => value
                .downcast::<Option<Inject<T>>>()
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

/// 宏为一个字段单态化生成的输入准备函数。
///
/// runtime 只把目标服务身份查为 [`ArenaServiceRef`]；此函数项保留 `T`，因此无须、
/// 也不能从 `TypeId` 反推字段的精确类型。
#[doc(hidden)]
pub type PrepareInput = fn(
    &mut ConstructionContext,
    InputPosition,
    Option<ArenaServiceRef>,
) -> Result<(), ActivationError>;

/// 为一个必选 concrete 字段将 Arena 地址写入构造 context。
#[doc(hidden)]
pub fn prepare_required<T>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError>
where
    T: Injectable,
{
    let input = input.ok_or(ActivationError::MissingInput { position })?;
    let pointer = input.cast::<T>(position)?;

    // SAFETY: `ArenaServiceRef::cast` verified the precise type and the reference can only
    // originate from a committed, stable Arena allocation.
    unsafe { context.insert_required_ptr(position, pointer) }
}

/// 为一个可选 concrete 字段将 Arena 地址或 `None` 写入构造 context。
#[doc(hidden)]
pub fn prepare_optional<T>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
) -> Result<(), ActivationError>
where
    T: Injectable,
{
    let pointer = input.map(|input| input.cast::<T>(position)).transpose()?;

    // SAFETY: if present, `pointer` was type-checked from a committed stable Arena entry.
    unsafe { context.insert_optional_ptr(position, pointer) }
}

/// 为没有 bind provider 的可选 trait 字段写入 `None`。
///
/// 这个函数不尝试把 thin concrete pointer 伪造成 `dyn Trait`。它只允许 `None`，因此
/// `Option<Inject<dyn Trait>>` 在没有匹配 `#[bind]` 时仍可安全地表示缺失依赖。
#[doc(hidden)]
pub fn prepare_optional_absent<T>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
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

    // SAFETY: no pointer is stored for the absent case, so no trait-object projection or Arena
    // address interpretation is required.
    unsafe { context.insert_optional_ptr::<T>(position, None) }
}

/// 用 `#[bind]` 生成的 concrete-to-trait projector 准备一个必选 trait 字段输入。
///
/// `Concrete` 和 `Trait` 由 bind 宏的单态化 wrapper 同时保留。首先校验 Arena 地址确实
/// 是 `Concrete`，然后调用 Rust 类型系统验证过的投影函数获得真实 trait-object vtable。
#[doc(hidden)]
pub fn prepare_bound_required<Concrete, Trait>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<(), ActivationError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    let input = input.ok_or(ActivationError::MissingInput { position })?;
    let concrete = input.cast::<Concrete>(position)?;
    let trait_pointer = project_bound_pointer(concrete, project);

    // SAFETY: `project_bound_pointer` created this fat pointer through a Rust-typed reference
    // projection from a committed `Concrete` Arena allocation.
    unsafe { context.insert_required_ptr(position, trait_pointer) }
}

/// 用 `#[bind]` 生成的 concrete-to-trait projector 准备一个可选 trait 字段输入。
#[doc(hidden)]
pub fn prepare_bound_optional<Concrete, Trait>(
    context: &mut ConstructionContext,
    position: InputPosition,
    input: Option<ArenaServiceRef>,
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

    // SAFETY: `Some` pointers originate from the same typed projection as the required path;
    // `None` contains no pointer and is safe for an absent optional dependency.
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
    // SAFETY: `ArenaServiceRef::cast::<Concrete>` has already verified the exact concrete type
    // and Arena allocations remain stable until all consumers are dropped.
    let concrete = unsafe { concrete.as_ref() };
    NonNull::from(project(concrete))
}

/// 由构造 adapter 返回的具体服务的 owning type-erased 表示。
///
/// 它保留 concrete [`ServiceType`]，并拥有实际分配；后续 runtime 可以据此存入
/// ready slot 并进行 bind projection。它没有暴露原始指针给业务代码。
type AnyService = Box<dyn Any + Send + Sync>;

/// 将尚未提交的 concrete 值移动进 Arena 已分配内存的单态化函数。
pub(crate) type ErasedTransferFn = unsafe fn(AnyService, *mut u8);

/// 析构某个已经转移至 Arena 的 concrete 值的单态化函数。
pub(crate) type ErasedDropFn = unsafe fn(*mut u8);

pub struct ErasedService {
    service_type: ServiceType,
    value: AnyService,
    layout: fn() -> Layout,
    transfer_into: ErasedTransferFn,
    drop_value: ErasedDropFn,
}

impl ErasedService {
    /// 擦除一个成功构造的 concrete service。
    pub fn new<T>(value: T) -> Self
    where
        T: Injectable,
    {
        Self {
            service_type: ServiceType::create::<T>(),
            value: Box::new(value),
            layout: Layout::new::<T>,
            transfer_into: transfer_into::<T>,
            drop_value: drop_value::<T>,
        }
    }

    /// 返回实际持有的 concrete service 类型。
    pub fn service_type(&self) -> ServiceType {
        self.service_type
    }

    /// 消费 type-erased service 并恢复其 concrete 类型。
    ///
    /// 这是隐藏 ABI 的受控边界，主要供 runtime 和聚焦测试验证 adapter 输出；常规
    /// 业务解析路径不应把 concrete value 从 container 中取走。
    pub fn downcast<T>(self) -> Result<T, Self>
    where
        T: Injectable,
    {
        let Self {
            service_type,
            value,
            layout,
            transfer_into,
            drop_value,
        } = self;

        match value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => Err(Self {
                service_type,
                value,
                layout,
                transfer_into,
                drop_value,
            }),
        }
    }

    pub(crate) fn layout(&self) -> Layout {
        (self.layout)()
    }

    pub(crate) fn drop_value(&self) -> ErasedDropFn {
        self.drop_value
    }

    /// 将唯一拥有的服务值移动到经过 layout 验证的 Arena 存储。
    ///
    /// # Safety
    ///
    /// `destination` 必须是未初始化、对齐且布局精确匹配此 `ErasedService` concrete
    /// 类型的一次性可写存储。
    pub(crate) unsafe fn transfer_into(self, destination: NonNull<u8>) {
        unsafe { (self.transfer_into)(self.value, destination.as_ptr()) };
    }
}

/// 把 `ErasedService::new::<T>` 保存的唯一所有权移动到已验证目标地址。
///
/// # Safety
///
/// `destination` 必须可写、正确对齐、尚未初始化且布局精确为 `T`。
unsafe fn transfer_into<T>(value: AnyService, destination: *mut u8)
where
    T: Injectable,
{
    let value = value
        .downcast::<T>()
        .expect("ErasedService transfer vtable must match its concrete value type");
    unsafe { destination.cast::<T>().write(*value) };
}

/// 析构经同一 `ErasedService` 虚表转移到 Arena 的 concrete 值。
///
/// # Safety
///
/// `destination` 必须指向恰好一个仍未析构的 `T` 值。
unsafe fn drop_value<T>(destination: *mut u8)
where
    T: Injectable,
{
    unsafe { std::ptr::drop_in_place(destination.cast::<T>()) };
}

/// 所有同步 provider 构造 adapter 的统一函数签名。
///
/// `#[injectable]` 的结构体字段构造和未来同步 `#[factory]` 都从同一个
/// [`ConstructionContext`] 取得预绑定的 `Inject<T>` 输入，并返回 owning
/// [`ErasedService`]。
pub type Constructor = fn(ConstructionContext) -> Result<ErasedService, ActivationError>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        arena::Arena,
        registration::{service_identifier::ServiceIdentifier, service_type::ServiceType},
    };

    struct RequiredDependency;
    struct OptionalDependency;

    struct Consumer {
        required: Inject<RequiredDependency>,
        optional: Option<Inject<OptionalDependency>>,
    }

    fn construct_consumer(
        mut context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        let required = context.take::<RequiredDependency>(InputPosition(0))?;
        let optional = context.take_optional::<OptionalDependency>(InputPosition(1))?;

        Ok(ErasedService::new(Consumer { required, optional }))
    }

    #[test]
    fn constructor_consumes_required_and_optional_tokens_and_erases_the_result() {
        let required_identifier =
            ServiceIdentifier::from(ServiceType::create::<RequiredDependency>());
        let optional_identifier =
            ServiceIdentifier::from(ServiceType::create::<OptionalDependency>());
        let mut arena = Arena::new();
        arena
            .insert(required_identifier, RequiredDependency)
            .expect("required dependency should commit to the arena");
        arena
            .insert(optional_identifier, OptionalDependency)
            .expect("optional dependency should commit to the arena");

        let mut context = ConstructionContext::new();
        prepare_required::<RequiredDependency>(
            &mut context,
            InputPosition(0),
            arena.lookup(required_identifier),
        )
        .expect("required input should be prepared from the arena");
        prepare_optional::<OptionalDependency>(
            &mut context,
            InputPosition(1),
            arena.lookup(optional_identifier),
        )
        .expect("optional input should be prepared from the arena");

        let constructor: Constructor = construct_consumer;
        let erased = constructor(context).expect("adapter should receive its bound inputs");

        assert_eq!(erased.service_type(), ServiceType::create::<Consumer>());

        let consumer = match erased.downcast::<Consumer>() {
            Ok(consumer) => consumer,
            Err(_) => panic!("adapter output should retain the concrete service"),
        };
        let Consumer { required, optional } = consumer;

        assert!(std::ptr::eq(
            &*required,
            arena.get::<RequiredDependency>().unwrap()
        ));
        assert!(optional.is_some());
    }
}
