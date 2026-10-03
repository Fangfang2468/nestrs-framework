//! 隐藏 `#[bind]` 回归入口的类型化投影生成。
//!
//! bind 的语义不是注册一份 trait-object 实例，也不是注册 provider：它只将具体服务的
//! 地址通过 Rust 类型系统投影为 `dyn Trait`，从而保存正确的 vtable。真实配对通过
//! 局部反射 marker 交给编译器选择，目标端回调只返回执行投影；它不寻找 provider、
//! 不物化泛型，也不会创建另一份实例或覆盖 provider 自身的 key。

use crate::{
    codegen::reflection::ident,
    protocol::{self, Marker},
};
use zyn::{syn, zyn};

/// 输出一条 trait 到 concrete 的 typed binding 注册。
///
/// 调用方负责保留原始 `impl Trait for Concrete` 并做属性/作用域/dyn-compatible
/// 校验；本 element 只生成与该 impl 同一展开位置的类型化 projector 和描述
/// binding callback。
#[zyn::element]
pub(crate) fn emit_bound_provider(
    service: syn::Type,
    interface: syn::Path,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    let reflection = crate::codegen::reflection::support(false, *binding_span);
    let reflection_module = ident(protocol::REFLECTION_MODULE, *binding_span);
    let binding_marker = ident(Marker::Binding.name(), *binding_span);
    let projector = ident("__nestrs_project_bound_service", *binding_span);
    let callback = ident("__nestrs_reflect_trait_binding", *binding_span);
    let service_ref = ident("service", *binding_span);
    let projected = ident("projected", *binding_span);
    let slot = ident("slot", *binding_span);
    let input = ident("input", *binding_span);
    let target = ident("target", *binding_span);
    zyn! {
        #[allow(clippy::unused_unit)]
        const _: () = {
            {{ reflection }}
            fn {{ projector }}(
                {{ service_ref }}: &{{ service }}
            ) -> &(dyn {{ interface }} + 'static) {
                let {{ projected }}: &(dyn {{ interface }} + 'static) = {{ service_ref }};
                {{ projected }}
            }

            #[allow(dead_code)]
            #[allow(clippy::needless_borrow)]
            fn {{ callback }}()
                -> ::nestrs_core::activation::adapter::ProjectionAdapter
            {
                {{ reflection_module }}::{{ binding_marker }}::<{{ service }}, dyn {{ interface }}>();
                ::nestrs_core::activation::adapter::ProjectionAdapter {
                    trait_type: ::nestrs_core::service::ServiceType::create::<
                        dyn {{ interface }}
                    >(),
                    concrete_type: ::nestrs_core::service::ServiceType::create::<
                        {{ service }}
                    >(),
                    project: (|
                        {{ slot }}: ::nestrs_core::activation::InputSlot,
                        {{ input }}: ::nestrs_core::activation::ErasedServiceRef,
                        {{ target }}: &mut ::nestrs_core::activation::ProjectionTarget<'_>,
                    | {
                        ::nestrs_core::activation::project_bound::<
                            {{ service }},
                            dyn {{ interface }},
                        >(
                            {{ slot }},
                            {{ input }},
                            {{ target }},
                            {{ projector }},
                        )
                    }) as ::nestrs_core::activation::ServiceProjector,
                }
            }

            ()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::{Render, syn};

    #[test]
    fn emits_a_binding_with_typed_projectors_without_a_separate_key_policy() {
        let rendered = EmitBoundProvider {
            binding_span: zyn::proc_macro2::Span::mixed_site(),
            service: syn::parse_str("ConcreteService").expect("service type should parse"),
            interface: syn::parse_str("Port").expect("trait path should parse"),
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string();

        assert!(rendered.contains("compiler_binding"));
        assert!(rendered.contains("ProjectionAdapter"));
        assert!(!rendered.contains("key_policy"));
        assert!(!rendered.contains("prepare_"));
        assert!(rendered.contains("project_bound"));
        assert!(rendered.contains("ProjectionTarget"));
        assert!(rendered.contains("ServiceProjector"));
        assert!(rendered.contains("ErasedServiceRef"));
        assert!(rendered.contains("InputSlot"));
        assert!(rendered.contains("ConcreteService"));
        assert!(rendered.contains("dyn Port"));
    }
}
