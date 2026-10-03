//! 将已经就绪的实例直接交付给调用者的类型化令牌槽位。
//!
//! 普通 trait 查询与延迟字段交付都只需要一个 `Injection<T>`，不需要普通构造输入
//! 使用的装箱载荷。这里借用调用者栈上的 `Option<Injection<T>>`，用 `Any::downcast_mut` 验证完整
//! 类型后写入。擦除的只是短暂借用，不擦除所有权、不延长生命周期，也不拼装裸指针。
//! 普通 preparer 和这条直接交付路径复用同一组令牌构造函数，确保 concrete 类型检查、
//! trait coercion 和真实实例 lease 的规则始终只有一份。

use std::{any::Any, ptr::NonNull};

use super::{ConstructionError, InputSlot};
use crate::{
    activation::{DependencyLease, ErasedServiceRef, Injection},
    service::Injectable,
};

/// 已选 provider 的无装箱类型化投影；目标只在这一次同步调用期间借出。
pub type ServiceProjector = for<'a> fn(
    InputSlot,
    ErasedServiceRef,
    &mut ProjectionTarget<'a>,
) -> Result<(), ConstructionError>;

/// 一次投影的私有接收端，指向调用者栈上的准确 `Option<Injection<T>>`。
///
/// 生成代码只能提交带有真实 lease 的 token，不能取得底层 `Any` 或构造接收端。
/// 无论 adapter 是否传播写入错误，重复写入或错误类型都会使整次交付失败。
pub struct ProjectionTarget<'a> {
    slot: InputSlot,
    value: &'a mut dyn Any,
    expected: &'static str,
    failure: Option<ConstructionError>,
}

impl ProjectionTarget<'_> {
    /// 精确类型匹配后才提交所有权；失败时 token 按普通 Rust 析构规则回滚。
    pub fn write<T>(&mut self, token: Injection<T>) -> Result<(), ConstructionError>
    where
        T: Injectable + ?Sized,
    {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        let error = match self.value.downcast_mut::<Option<Injection<T>>>() {
            None => ConstructionError::InputTypeMismatch {
                slot: self.slot,
                expected: self.expected,
                actual: std::any::type_name::<T>(),
            },
            Some(value) if value.is_some() => {
                ConstructionError::SlotAlreadyPrepared { slot: self.slot }
            }
            Some(value) => {
                *value = Some(token);
                return Ok(());
            }
        };
        self.failure = Some(error.clone());
        Err(error)
    }
}

/// 从共享计划指定的投影函数取得准确令牌，成功路径不分配临时堆载荷。
///
/// 投影只能改变同一个实例的类型化视图，不能把另一个同类型实例替换进结果。
/// 普通查询随后会释放令牌自己的 lease，并把引用的存活期交给实际 owner；因此
/// 类型正确还不够，必须确认令牌属于 runtime 已发布的那个实例。延迟交付复用同一
/// 检查，确保它缓存的也是已选任务的结果，不允许安全 adapter 改变计划中的实例身份。
pub(crate) fn project_token<T>(
    slot: InputSlot,
    lease: DependencyLease,
    projector: ServiceProjector,
) -> Result<Injection<T>, ConstructionError>
where
    T: Injectable + ?Sized,
{
    let mut token: Option<Injection<T>> = None;
    let mut target = ProjectionTarget {
        slot,
        value: &mut token,
        expected: std::any::type_name::<T>(),
        failure: None,
    };
    projector(slot, lease.erased_ref(), &mut target)?;
    // adapter 返回 Ok 不等于交付完成：吞掉的写入错误、静默不写入都必须在这里拒绝。
    if let Some(error) = target.failure {
        return Err(error);
    }
    let token = token.ok_or(ConstructionError::UnfilledSlot { slot })?;
    if !token.lease().ptr_eq(&lease) {
        return Err(ConstructionError::ProjectionOwnerMismatch { slot });
    }
    Ok(token)
}

/// 将 concrete 实例直接投影到调用者的令牌槽位。
pub fn project_required<T>(
    slot: InputSlot,
    input: ErasedServiceRef,
    target: &mut ProjectionTarget<'_>,
) -> Result<(), ConstructionError>
where
    T: Injectable + ?Sized,
{
    target.write(required_token::<T>(slot, input)?)
}

/// 复用 rustc 检查过的 coercion，直接交付完整的 trait-object 令牌。
pub fn project_bound<Concrete, Trait>(
    slot: InputSlot,
    input: ErasedServiceRef,
    target: &mut ProjectionTarget<'_>,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<(), ConstructionError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    target.write(bound_token::<Concrete, Trait>(slot, input, project)?)
}

/// 普通构造输入与延迟输入共同使用的准确地址恢复边界。
pub(super) fn required_token<T>(
    slot: InputSlot,
    input: ErasedServiceRef,
) -> Result<Injection<T>, ConstructionError>
where
    T: Injectable + ?Sized,
{
    let (pointer, lease) = cast_input::<T>(slot, input)?;
    // SAFETY: cast_input 已按准确 T 检查地址类型，lease 持有同一不可移动的真实实例。
    Ok(unsafe { Injection::from_service_ptr(pointer, lease) })
}

/// trait 交付也始终先恢复准确 concrete 类型，再执行真实 Rust 类型化投影。
pub(super) fn bound_token<Concrete, Trait>(
    slot: InputSlot,
    input: ErasedServiceRef,
    project: for<'a> fn(&'a Concrete) -> &'a Trait,
) -> Result<Injection<Trait>, ConstructionError>
where
    Concrete: Injectable,
    Trait: Injectable + ?Sized,
{
    let (concrete, lease) = cast_input::<Concrete>(slot, input)?;
    // SAFETY: cast_input 保证 concrete 的准确类型，当前 lease 保活稳定实例；高阶借用
    // 签名限制投影结果的存活期，返回的完整地址由真实 coercion 产生而非伪造 vtable。
    let pointer = NonNull::from(project(unsafe { concrete.as_ref() }));
    // SAFETY: pointer 来自上面的合法投影，令牌接收同一 concrete 实例的强 lease。
    Ok(unsafe { Injection::from_service_ptr(pointer, lease) })
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

#[cfg(test)]
#[path = "../../../tests/unit/activation/construction/projection.rs"]
mod tests;
