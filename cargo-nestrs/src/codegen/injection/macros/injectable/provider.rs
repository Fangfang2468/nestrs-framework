//! `#[injectable]` 的 `Provider` 生成。
//!
//! 此模块只消费字段分析结果，不参与字段改写或构造代码生成。这样 provider 身份、
//! 字段 key、可选性和 DI 输入位置都来自与字段改写相同的
//! `FieldSpec`，不会重新解析已被清除的 marker。

use crate::codegen::constructor::ConstructorMode;
use crate::codegen::injection::render::{
    EmitCompilerKey, EmitDependencyRequest, EmitPlanProvider, RenderCleanupHook,
};
use crate::{
    codegen::reflection,
    protocol::{self, Marker},
};

use super::{
    config::InjectableConfig,
    field_analyze::{AnalyzedFields, FieldStrategy},
};
use zyn::{
    quote::{ToTokens, quote},
    zyn,
};

/// 生成由编译器收集的 class provider 描述回调。
///
/// 此 element 只生成类型化描述，不负责构造 adapter 或匿名作用域。调用方
/// 必须将它和 `GenerateInjectableConstructor` 放在同一个匿名 `const` 中，才能把
/// 词法私有的 `__nestrs_construct` 函数指针写入 provider。
#[zyn::element]
pub(crate) fn collect_injectable_provider(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
    lazy: Option<bool>,
    source: crate::codegen::source::ProviderOrigin,
    mode: ConstructorMode,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let service_type = quote!(#service);
    let reflection_module = reflection::ident(protocol::REFLECTION_MODULE, *binding_span);
    let provider_marker = reflection::ident(Marker::Provider.name(), *binding_span);
    let origins = source.render(*binding_span);
    let callback = reflection::ident("__nestrs_reflect_provider", *binding_span);
    let construct = reflection::ident("__nestrs_construct", *binding_span);

    zyn! {
        #[allow(dead_code)]
        fn {{ callback }}() -> ::nestrs_core::activation::adapter::ActivationAdapter {
            {{ reflection_module }}::{{ provider_marker }}::<{{ service_type.clone() }}>(
                @EmitCompilerKey(key = config.key.clone(), binding_span = *binding_span)
            );
            {{ origins }}
            @EmitPlanProvider(
                binding_span = *binding_span,
                service_type = service_type.clone(),
                key = config.key.clone(),
                lifetime = config.lifetime,
                primary = *primary,
                lazy = *lazy,
            )
            ::nestrs_core::activation::adapter::ActivationAdapter {
                    @EmitClassProviderFields(
                        binding_span = *binding_span,
                        analysis = analysis.clone(),
                        config = config.clone(),
                        service_type = service_type.clone(),
                        mode = *mode,
                    )
                    constructor: ::nestrs_core::activation::adapter::Constructor::Class({{ construct }}),
            }
        }
    }
}

/// 输出一个 class provider 在身份、依赖与公共属性上的字段。
///
/// 常规闭合服务把这些字段包在 描述回调 中；开放泛型服务则由
/// `ProviderDefinition::provider()` 使用完全相同的字段。构造 adapter 有不同的
/// 词法可见性需求，故由调用方在这个 element 的输出之后单独提供 `constructor`。
#[zyn::element]
pub(crate) fn emit_class_provider_fields(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    service_type: zyn::TokenStream,
    mode: ConstructorMode,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    let cleanup = config.cleanup.clone();
    let dependencies = match &analysis.constructor_helpers {
        Some(helpers) => {
            zyn::syn::Ident::new(&helpers.dependencies, zyn::proc_macro2::Span::call_site())
        }
        None => reflection::ident(protocol::constructor::DEPENDENCIES, *binding_span),
    };
    zyn! {
        service_type: ::nestrs_core::service::ServiceType::create::<{{ service_type.clone() }}>(),
        cleanup: @RenderCleanupHook(cleanup = cleanup.clone()),
        inputs: @match (*mode) {
            ConstructorMode::Deferred => {
                if false {
                    {{ service_type.clone() }}::{{ dependencies.clone() }}()
                } else {
                    @RenderFieldInputs(analysis = analysis.clone(), binding_span = *binding_span)
                }
            }
            ConstructorMode::Automatic => {
                { @RenderFieldInputs(analysis = analysis.clone(), binding_span = *binding_span) }
            }
            ConstructorMode::Explicit => {
                { {{ service_type.clone() }}::{{ dependencies.clone() }}() }
            }
        },
    }
}

#[zyn::element]
fn render_field_inputs(
    analysis: AnalyzedFields,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    // 两个候选都先交给标准 Rust 名称解析；driver 在 HIR 降低前按真实服务身份
    // 选择一种构造模式。未选中的自动 Default/value 不参与类型检查或求值。
    let field_mode = analysis
        .specs
        .iter()
        .any(|spec| !matches!(spec.strategy, FieldStrategy::Default));
    let original_input = analysis.item.to_token_stream().to_string();
    let field_mode_ident = reflection::ident("__nestrs_constructor_field_mode", *binding_span);
    let original_input_ident = reflection::ident("__nestrs_constructor_input", *binding_span);

    zyn! {
            let {{ field_mode_ident }} = {{ field_mode }};
            let {{ original_input_ident }} = {{ original_input }};
            ::std::vec![
            @for (spec in analysis.specs.iter()) {
                @if (spec.is_injected()) {
                    @EmitDependencyRequest(request = spec.dependency_request(), binding_span = *binding_span),
                }
            }
            ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::injection::{
        macros::injectable::field_analyze::{AnalyzedFields, FieldSpec, collect_field_specs},
        macros_attrs::{lifetime::ServiceLifetime, service_key::ServiceKeySpec},
    };
    use zyn::{Render, syn};

    fn render_provider(
        item: syn::ItemStruct,
        specs: Vec<FieldSpec>,
        config: InjectableConfig,
        primary: bool,
        lazy: Option<bool>,
    ) -> String {
        CollectInjectableProvider {
            binding_span: zyn::proc_macro2::Span::mixed_site(),
            source: crate::codegen::source::ProviderOrigin::from_args(
                item.ident.clone(),
                &syn::parse_quote!(),
                None,
            ),
            analysis: AnalyzedFields {
                item,
                specs,
                constructor_helpers: None,
            },
            config,
            primary,
            lazy,
            mode: ConstructorMode::Deferred,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn selected_provider_inputs_do_not_render_the_other_constructor_mode() {
        for mode in [
            ConstructorMode::Deferred,
            ConstructorMode::Automatic,
            ConstructorMode::Explicit,
        ] {
            let analysis = super::super::field_analyze::analyze_fields(syn::parse_quote! {
                struct Service { #[inject] dependency: Dependency }
            })
            .unwrap();
            let rendered = CollectInjectableProvider {
                binding_span: zyn::proc_macro2::Span::mixed_site(),
                source: crate::codegen::source::ProviderOrigin::from_args(
                    analysis.item.ident.clone(),
                    &syn::parse_quote!(),
                    None,
                ),
                analysis,
                config: InjectableConfig {
                    lifetime: ServiceLifetime::Singleton,
                    key: None,
                    cleanup: None,
                },
                primary: false,
                lazy: None,
                mode,
            }
            .render(&zyn::Input::default());
            syn::parse2::<syn::ItemFn>(rendered.tokens().clone()).unwrap();
            let tokens = rendered.tokens().to_string();
            assert_eq!(
                tokens.contains("if false"),
                mode == ConstructorMode::Deferred,
                "{tokens}"
            );
            assert_eq!(
                tokens.contains("__nestrs_constructor_dependencies"),
                mode != ConstructorMode::Automatic,
                "{tokens}"
            );
            assert_eq!(
                tokens.contains("compiler_plan_input"),
                mode != ConstructorMode::Explicit,
                "{tokens}"
            );
            assert_eq!(
                tokens.contains("__nestrs_constructor_input"),
                mode != ConstructorMode::Explicit,
                "{tokens}"
            );
        }
    }

    #[test]
    fn emits_class_provider_and_dependency_specs_from_the_shared_analysis() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Controller {
                #[inject]
                database: Database,
                #[value("fixed")]
                name: &'static str,
                #[inject(7)]
                audit: Option<dyn Audit>,
            }
            "#,
        )
        .expect("test input should parse");
        let specs = collect_field_specs(&item.fields).expect("fields should be valid");
        let config = InjectableConfig {
            lifetime: ServiceLifetime::Scoped,
            key: Some(ServiceKeySpec::named("controller")),
            cleanup: None,
        };

        let output = render_provider(item, specs, config, true, None);

        assert!(output.contains("compiler_provider"));
        assert!(output.contains("Constructor :: Class"));
        assert!(output.contains("Constructor :: Class (__nestrs_construct)"));
        assert!(output.contains("compiler_plan_provider :: < Controller , 1u8 , true , 0u8 >"));
        assert!(output.contains("CompilerKey :: Named"));
        assert!(output.contains("\"controller\""));
        assert!(output.contains("compiler_plan_input :: < Database , 0usize , false , false >"));
        assert!(output.contains("compiler_plan_input :: < dyn Audit , 1usize , true , false >"));
        assert!(output.contains("CompilerKey :: Indexed (7usize)"));
        assert!(!output.contains("DependencyRequest"));
        assert!(!output.contains("ProviderSource"));
        assert!(!output.contains("ServiceIdentifier"));
    }
}
