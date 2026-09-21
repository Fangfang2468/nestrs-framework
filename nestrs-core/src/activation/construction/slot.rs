//! 构造输入的固定槽位标识。

/// 宏生成 adapter 与 provider descriptor 共享的固定输入槽位标识。
///
/// 槽位的有效范围由后续 [`super::inputs::InputBuffer`] 的长度决定；调用方不能依靠
/// 构造该值绕过范围检查。
#[doc(hidden)]
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InputSlot(usize);

impl InputSlot {
    /// 创建一个由宏生成的固定输入槽位。
    #[inline]
    pub const fn new(index: usize) -> Self {
        Self(index)
    }

    #[inline]
    pub(super) const fn index(self) -> usize {
        self.0
    }
}
