use std::{marker::PhantomData, ops::Deref, ptr::NonNull};

use crate::registration::injectable::Injectable;

/// `#[injectable]` 字段令牌的默认访问来源。
///
/// 此标记仅用于固定宏 ABI；业务代码不应命名、构造或依赖它。
#[doc(hidden)]
pub struct FieldInject(PhantomData<()>);

/// `#[factory]` 参数令牌的激活期访问来源。
///
/// 未来 factory 宏会使用该生命周期标记，使参数令牌不能安全地逃逸到比当前激活期更长的
/// 服务或任务中。当前只定义 ABI，不接入 factory 构造流程。
#[doc(hidden)]
pub struct FactoryParameter<'frame>(PhantomData<&'frame mut ()>);

/// 宏 ABI 预留的只读依赖令牌。
///
/// 对下游宏展开稳定暴露的形状是 `Inject<T, Access = FieldInject>`。它只授予目标
/// 服务的共享访问，不提供构造、复制、可变访问或所有权提取入口。
///
/// 当前实例化 Arena 在提交依赖时为其分配稳定地址，并保证消费者先于依赖析构。令牌只
/// 保存该地址，不拥有服务，也不会延长 Arena 生命周期；因此它只能由宏生成 constructor
/// 在同一个 Arena 中构造并存入服务字段。
pub struct Inject<T: ?Sized, Access = FieldInject> {
    /// 已提交到当前 Arena 的精确服务地址。
    ptr: NonNull<T>,

    /// 记录目标类型的 drop-check 关系，但不取得其所有权。
    _marker: PhantomData<T>,

    /// 区分可保存的字段令牌和受激活期约束的 factory 参数令牌。
    _access: PhantomData<Access>,
}

impl<T: ?Sized> Inject<T, FieldInject>
where
    T: Injectable,
{
    /// 从实例化 Arena 中已验证的稳定服务地址创建字段 token。
    ///
    /// # Safety
    ///
    /// `ptr` 必须指向当前 Arena 已提交且精确为 `T` 的服务；返回 token 必须立即移入
    /// 同一 Arena 将持有的宏生成字段，不能被构造成独立逃逸值。
    pub(crate) unsafe fn from_field_ptr(ptr: NonNull<T>) -> Self {
        Self {
            ptr,
            _marker: PhantomData,
            _access: PhantomData,
        }
    }
}

impl<T: ?Sized, Access> Deref for Inject<T, Access>
where
    T: Injectable,
{
    type Target = T;

    /// 只读地访问已注入的服务。
    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: token only originates from a committed stable Arena entry. The Arena keeps
        // dependencies alive until all later-committed consumers (including this token) drop.
        unsafe { self.ptr.as_ref() }
    }
}

// `Inject<T>` 只能由隐藏 ABI 为 `T: Injectable` 构造，因此已签发 token 指向的目标始终
// 满足 `Send + Sync + 'static`。不能在下面添加 `T: Send` / `T: Sync` bound：互相注入的
// 服务会在 Rust 自动 trait 推导阶段递归，遮蔽运行时的受控重入错误。
//
// Token 不提供安全构造、复制、可变借用、所有权提取或裸指针导出；Access 也只通过
// PhantomData 表达来源边界，不持有额外的可访问数据。
unsafe impl<T: ?Sized, Access> Send for Inject<T, Access> {}
unsafe impl<T: ?Sized, Access> Sync for Inject<T, Access> {}

#[cfg(test)]
mod tests {
    use super::Inject;

    use crate::{
        arena::Arena,
        construction::{ConstructionContext, InputPosition, prepare_required},
        registration::{service_identifier::ServiceIdentifier, service_type::ServiceType},
    };

    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    struct First {
        _second: Inject<Second>,
    }

    struct Second {
        _first: Inject<First>,
    }

    #[test]
    fn inject_dereferences_to_its_service() {
        let identifier = ServiceIdentifier::from(ServiceType::create::<String>());
        let mut arena = Arena::new();
        arena
            .insert(identifier, String::from("database"))
            .expect("string service should commit");

        let mut context = ConstructionContext::new();
        prepare_required::<String>(&mut context, InputPosition(0), arena.lookup(identifier))
            .expect("arena service should prepare an Inject token");
        let inject = context
            .take::<String>(InputPosition(0))
            .expect("prepared token should have the expected type");

        assert_eq!(inject.len(), 8);
        assert_eq!(&*inject, "database");
    }

    #[test]
    fn inject_keeps_explicit_send_and_sync_for_recursive_dependencies() {
        assert_send::<Inject<String>>();
        assert_sync::<Inject<String>>();
        assert_send::<First>();
        assert_sync::<First>();
    }
}
