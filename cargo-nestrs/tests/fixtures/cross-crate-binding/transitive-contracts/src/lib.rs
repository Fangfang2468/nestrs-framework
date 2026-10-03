//! 模拟由普通依赖带入的 trait metadata，不依赖 Nestrs。

/// 没有重导出的公共 trait；消费者在 metadata 中看见它不代表能写出源码路径。
pub trait UnexportedCapability: Send + Sync {}
impl<T: Send + Sync> UnexportedCapability for T {}

pub struct HiddenArgument;

/// trait 有重导出，但闭合实参没有；两部分都必须可命名才能生成 adapter。
pub trait WithHiddenArgument<T>: Send + Sync {}
impl<T: Send + Sync> WithHiddenArgument<HiddenArgument> for T {}

pub trait ExposedMarker: Send + Sync {
    fn value(&self) -> usize;
}

pub trait PublicCapability: Send + Sync {
    fn exposed_value(&self) -> usize;
    fn identity(&self) -> usize;
}

// 来自传递 crate 的真实业务 blanket impl 不能被“一律忽略传递依赖”的规则丢掉。
impl<T: ExposedMarker> PublicCapability for T {
    fn exposed_value(&self) -> usize {
        self.value()
    }

    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}
