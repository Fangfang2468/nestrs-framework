//! `#[bind]` 的 `TraitBinding` 注册生成。
//!
//! bind 的语义不是注册一份 trait-object 实例，也不是注册 provider：它只将具体服务的
//! Arena 地址通过 Rust 类型系统投影为 `dyn Trait`，从而保存正确的 vtable，因此进入
//! 独立的绑定切片。请求 key 在未来的 provider 选择阶段按 `InheritRequestedKey`
//! 继承到 concrete 服务 token。

use zyn::{syn, zyn};

/// 输出一条 trait 到 concrete 的 typed binding 注册。
///
/// 调用方负责保留原始 `impl Trait for Concrete` 并做属性/作用域/dyn-compatible
/// 校验；本 element 只生成与该 impl 同一展开位置的类型化 projector 和 linkme
/// binding callback。
#[zyn::element]
pub(crate) fn emit_bound_provider(
    service: syn::Type,
    interface: syn::Path,
) -> zyn::TokenStream {
    zyn! {
        const _: () = {
            fn __nestrs_project_bound_service(
                service: &{{ service }}
            ) -> &(dyn {{ interface }} + 'static) {
                let projected: &(dyn {{ interface }} + 'static) = service;
                projected
            }

            fn __nestrs_prepare_bound_required(
                context: &mut ::nestrs_core::__private::ConstructionContext,
                position: ::nestrs_core::__private::InputPosition,
                input: ::core::option::Option<::nestrs_core::__private::ArenaServiceRef>,
            ) -> ::core::result::Result<(), ::nestrs_core::__private::ActivationError> {
                ::nestrs_core::__private::prepare_bound_required::<
                    {{ service }},
                    dyn {{ interface }},
                >(
                    context,
                    position,
                    input,
                    __nestrs_project_bound_service,
                )
            }

            fn __nestrs_prepare_bound_optional(
                context: &mut ::nestrs_core::__private::ConstructionContext,
                position: ::nestrs_core::__private::InputPosition,
                input: ::core::option::Option<::nestrs_core::__private::ArenaServiceRef>,
            ) -> ::core::result::Result<(), ::nestrs_core::__private::ActivationError> {
                ::nestrs_core::__private::prepare_bound_optional::<
                    {{ service }},
                    dyn {{ interface }},
                >(
                    context,
                    position,
                    input,
                    __nestrs_project_bound_service,
                )
            }

            #[::nestrs_core::__private::linkme::distributed_slice(
                ::nestrs_core::__private::REFLECTED_BINDINGS
            )]
            #[linkme(crate = ::nestrs_core::__private::linkme)]
            fn __nestrs_reflect_trait_binding()
                -> ::nestrs_core::__private::TraitBinding
            {
                ::nestrs_core::__private::TraitBinding {
                    trait_type: ::nestrs_core::registration::service_type::ServiceType::create::<
                        dyn {{ interface }}
                    >(),
                    concrete_type: ::nestrs_core::registration::service_type::ServiceType::create::<
                        {{ service }}
                    >(),
                    key_policy: ::nestrs_core::__private::BoundKeyPolicy::InheritRequestedKey,
                    prepare_required: __nestrs_prepare_bound_required
                        as ::nestrs_core::__private::PrepareInput,
                    prepare_optional: __nestrs_prepare_bound_optional
                        as ::nestrs_core::__private::PrepareInput,
                    source: ::nestrs_core::registration::service_source::ServiceSource::new(
                        file!(),
                        line!(),
                        column!(),
                    ),
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
    fn emits_a_bound_provider_with_typed_projectors_and_inherited_key_policy() {
        let rendered = EmitBoundProvider {
            service: syn::parse_str("ConcreteService").expect("service type should parse"),
            interface: syn::parse_str("Port").expect("trait path should parse"),
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string();

        assert!(rendered.contains("REFLECTED_BINDINGS"));
        assert!(rendered.contains("TraitBinding"));
        assert!(rendered.contains("BoundKeyPolicy :: InheritRequestedKey"));
        assert!(rendered.contains("prepare_bound_required"));
        assert!(rendered.contains("prepare_bound_optional"));
        assert!(rendered.contains("ConcreteService"));
        assert!(rendered.contains("dyn Port"));
    }
}
