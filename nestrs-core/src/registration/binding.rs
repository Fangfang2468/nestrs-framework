//! 显式绑定与编译器生成的 concrete-to-trait 投影描述。
//!
//! binding 只把同一 concrete 实例投影成接口，不注册另一份 Provider，也不构造实例。
//! 接口路由继承 concrete Provider 的精确 key；这是统一规则，不是可配置的策略。

use crate::{
    activation::InputPreparer,
    registration::dependency::ClosedProviderCallback,
    service::{ServiceSource, ServiceType},
};

/// 将一个 concrete provider 的稳定地址投影为 trait-object 输入的规则。
#[derive(Debug, Clone, Copy)]
pub struct TraitBinding {
    pub trait_type: ServiceType,
    pub concrete_type: ServiceType,
    /// 已知闭合 self 类型的可选蓝图；factory-only 类型不要求 ProviderDefinition。
    /// 图编译器在绑定校验前展开它，避免仅查询 dyn Trait 时遗漏其泛型实现。
    pub materialize: Option<ClosedProviderCallback>,
    /// 必选依赖使用的类型化投影；由真实 Rust coercion 生成 trait-object 元数据。
    pub prepare_required: InputPreparer,
    /// 可选依赖使用的类型化投影；缺席由对应 preparer 写入合法的 None。
    pub prepare_optional: InputPreparer,
    pub source: ServiceSource,
}
