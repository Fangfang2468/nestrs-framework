//! 编译器生成的每入口注册清单。这里没有全局注册表或链接段扫描。

use super::{binding::TraitBinding, provider::Provider, root::RootDeclaration};

/// 当前构图拥有的注册描述。前两类是实际声明，后两类是按需求启用的能力目录；
/// roots 是调用点已知类型。所有 Vec 都只存描述，不代表服务已经构造或可直接使用。
#[derive(Default)]
pub struct RegistrySnapshot {
    pub providers: Vec<Provider>,
    pub bindings: Vec<TraitBinding>,
    pub roots: Vec<RootDeclaration>,
    /// 预编译的合法投影，仅当根/依赖需要该接口时加入实际 binding 集合。
    pub automatic_bindings: Vec<TraitBinding>,
    /// 已知闭合类型的被动 Provider 蓝图，不因为进入目录就自动注册。
    pub blueprints: Vec<RootDeclaration>,
}

// 该符号由编译器在最终 binary/test 中生成且只定义一次。它用真实 typed
// callback 填充当前调用者拥有的快照，不执行任何服务构造或安装可变全局状态。
#[cfg(nestrs_compiler)]
unsafe extern "Rust" {
    fn __nestrs_registry_v1(output: *mut ());
}

/// 每次构图独立收集一次，冻结图之后不再调用入口或读取这些描述。
pub fn collect() -> RegistrySnapshot {
    #[allow(unused_mut)]
    let mut snapshot = RegistrySnapshot::default();
    #[cfg(nestrs_compiler)]
    // SAFETY: 匹配版本的 driver 只生成一个 ABI-v1 入口，且写入只能经过下面的类型化
    // helper。当前栈上 snapshot 在整个调用期间有效，地址不会被保存到全局状态。
    unsafe {
        __nestrs_registry_v1((&mut snapshot as *mut RegistrySnapshot).cast());
    }
    snapshot
}

macro_rules! append {
    ($name:ident, $field:ident, $value:ty) => {
        /// # Safety
        /// `output` 必须指向 collect 传入的唯一、仍存活的 RegistrySnapshot。
        #[allow(dead_code)] // 调用来自编译器生成的 MIR，普通 Rust 源码没有引用。
        pub unsafe fn $name(output: *mut (), value: $value) {
            // SAFETY: 只有经过审核的版本化编译器入口会收到并同步使用此地址。
            unsafe { &mut *output.cast::<RegistrySnapshot>() }
                .$field
                .push(value);
        }
    };
}

append!(registry_push_provider, providers, Provider);
append!(registry_push_binding, bindings, TraitBinding);
append!(registry_push_root, roots, RootDeclaration);
append!(
    registry_push_automatic_binding,
    automatic_bindings,
    TraitBinding
);
append!(registry_push_blueprint, blueprints, RootDeclaration);

/// CLI 图诊断入口只编译静态图并序列化描述，不构造 Provider，也不执行业务 main。
pub fn dependency_graph_json() -> Result<String, String> {
    crate::graph::GraphCompiler::compile_static()
        .map(|graph| crate::graph::snapshot(&graph).to_string())
        .map_err(|error| format!("DI 依赖图验证失败: {error}"))
}
