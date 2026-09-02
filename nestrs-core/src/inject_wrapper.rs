use std::{marker::PhantomData, ptr::NonNull};

/// 宏 ABI 预留的只读依赖令牌。
///
/// 令牌的构造、访问和生命周期管理将在激活运行时落地后实现；当前仅固定其
/// `Inject<T>` 类型形状，供后续宏展开保持兼容。
pub struct Inject<T: ?Sized> {
    /// 已解析依赖的稳定地址。
    #[allow(dead_code)]
    ptr: NonNull<T>,

    /// 保留目标类型的所有权与自动 trait 语义。
    _marker: PhantomData<T>,
}
