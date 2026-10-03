//! 仅供测试 oracle 使用的旧注册快照协议，生产执行计划不依赖此模块。
//! 没有全局注册表或链接段扫描；真实生产装配入口位于 graph::plan。

use super::{binding::TraitBinding, provider::Provider, root::RootDeclaration};

/// 当前构图拥有的注册描述。前两类是实际声明，后两类是按需求启用的能力目录；
/// roots 是调用点已知类型。所有 Vec 都只存描述，不代表服务已经构造或可直接使用。
#[derive(Default)]
pub struct RegistrySnapshot {
    /// 最终可执行入口所属 package 的启动配置；依赖库只贡献声明，不覆盖宿主设置。
    /// 普通 Cargo 的隔离测试没有编译器入口，因此仍使用 Lazy / 32 的缺省值。
    pub options: crate::ServiceProviderOptions,
    pub providers: Vec<Provider>,
    pub bindings: Vec<TraitBinding>,
    pub roots: Vec<RootDeclaration>,
    /// 预编译的合法投影，仅当根/依赖需要该接口时加入实际 binding 集合。
    pub automatic_bindings: Vec<TraitBinding>,
    /// 已知闭合类型的被动 Provider 蓝图，不因为进入目录就自动注册。
    pub blueprints: Vec<RootDeclaration>,
}
