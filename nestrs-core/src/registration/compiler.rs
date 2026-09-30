//! 供版本化编译器适配器读取的真实类型标记。
//!
//! 生成代码只在注册描述中插入这些调用。它们不持有运行期状态，也不注册、构造或查询
//! 服务；空函数体并非待补实现。调用需要保留在编码 MIR 中，使下游在单态化之前仍能
//! 分析闭合泛型与私有字段依赖；LLVM 后续可以消除运行期的空调用。
//!
//! 每种标记保留独立函数身份，让 driver 在宏展开及类型检查之后区分 Provider、实际
//! 请求、显式 binding 和被动投影能力，而不依赖源码名字或猜测泛型实参。

/// 编译器可以直接识别的静态 key。与真实类型一同出现，使精确 type/key 的显式
/// Provider 优先于蓝图；它不分配运行期 ServiceKey，也不增加注册状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompilerKey {
    Default,
    Named(&'static str),
    Indexed(usize),
}

#[inline(never)]
/// 标识一份实际 Provider 声明及其 key。
pub const fn compiler_provider<T: ?Sized>(_key: CompilerKey) {
    let _ = core::marker::PhantomData::<T>;
}

#[inline(never)]
/// 标识实际查询/依赖需求，可以激活对应接口的被动投影能力。
pub const fn compiler_request<T: ?Sized>() {
    let _ = core::marker::PhantomData::<T>;
}

/// 已知闭合 Provider 蓝图能力，本身不代表实际请求。
#[inline(never)]
pub const fn compiler_blueprint<T: ?Sized>() {
    let _ = core::marker::PhantomData::<T>;
}

#[inline(never)]
/// 保留构造输入槽位与真实依赖类型，支持下游沿私有字段路径发现闭合蓝图。
pub const fn compiler_dependency<T: ?Sized, const SLOT: usize>() {
    let _ = core::marker::PhantomData::<T>;
}

#[inline(never)]
/// 保留从已知锚点到私有依赖类型的编译期路径，不在运行期逐层遍历。
pub const fn compiler_blueprint_path<Anchor: ?Sized, Path>() {
    let _ = (
        core::marker::PhantomData::<Anchor>,
        core::marker::PhantomData::<Path>,
    );
}

#[inline(never)]
/// 标识显式 concrete-to-trait 投影声明，重复显式 pair 仍是图错误。
pub const fn compiler_binding<C: ?Sized, I: ?Sized>() {
    let _ = (
        core::marker::PhantomData::<C>,
        core::marker::PhantomData::<I>,
    );
}

/// 已预编译的投影能力；自身不请求任一类型，也不激活 concrete 的 Provider 蓝图。
#[inline(never)]
pub const fn compiler_automatic_binding<C: ?Sized, I: ?Sized>() {
    let _ = (
        core::marker::PhantomData::<C>,
        core::marker::PhantomData::<I>,
    );
}
