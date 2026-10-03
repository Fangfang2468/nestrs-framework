//! `#[injectable]` 隐藏构造 adapter 的生成。
//!
//! 此处只定义构造函数本身。`registration` 将它与 provider 的执行 adapter
//! 放入同一匿名 `const` 作用域，由 `ActivationAdapter` 持有的 `Constructor::Class`
//! 引用函数指针，同时不把 helper 暴露为结构体的 inherent method。

use super::{
    field_analyze::{AnalyzedFields, FieldSpec, FieldStrategy},
    field_initialization::RewriteValueField,
};
use crate::codegen::constructor::ConstructorMode;
use zyn::{
    quote::quote,
    syn::{self, Fields, ItemStruct, ext::IdentExt},
    zyn,
};

/// 输出一个仅供同一匿名注册作用域使用的构造 adapter。
///
/// 该 element 不自行添加 `const` 包裹。若作为顶层 sibling 输出，provider 就无法
/// 词法引用 `__nestrs_construct`；因此只能由 `EmitInjectableRegistration` 嵌入。
#[zyn::element]
pub(crate) fn generate_injectable_constructor(
    analysis: AnalyzedFields,
    mode: ConstructorMode,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let context = context_identifier(&analysis.item, *binding_span);
    let context_binding = context_binding(analysis, &context);
    let service = quote!(#service);

    zyn! {
        fn __nestrs_construct(
            {{ context_binding }}
        ) -> ::core::result::Result<
            ::nestrs_core::activation::ErasedService,
            ::nestrs_core::activation::ConstructionError,
        > {
            @RenderConstructorBody(
                analysis = analysis.clone(),
                service = service.clone(),
                context = context.clone(),
                mode = *mode,
            )
        }
    }
}

/// 输出开放泛型 `ProviderDefinition` 使用的无捕获构造 closure。
///
/// 它位于 trait 方法内部，因此 `Self` 已是由注入点单态化的服务类型；不像闭合
/// component 的 编译器注册，这里绝不能生成一个全局命名函数或把开放 provider
/// 放进 distributed slice。
#[zyn::element]
pub(crate) fn generate_generic_injectable_constructor(
    analysis: AnalyzedFields,
    mode: ConstructorMode,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    let context = context_identifier(&analysis.item, *binding_span);
    let context_binding = context_binding(analysis, &context);
    let service = quote!(Self);

    zyn! {
        |{{ context_binding }}| -> ::core::result::Result<
            ::nestrs_core::activation::ErasedService,
            ::nestrs_core::activation::ConstructionError,
        > {
            @RenderConstructorBody(
                analysis = analysis.clone(),
                service = service.clone(),
                context = context.clone(),
                mode = *mode,
            )
        }
    }
}

/// IDE 已知选择时只生成选中的构造体；普通编译仍把两个候选交给真实名称解析。
#[zyn::element]
fn render_constructor_body(
    analysis: AnalyzedFields,
    service: zyn::TokenStream,
    context: syn::Ident,
    mode: ConstructorMode,
) -> zyn::TokenStream {
    let activate = crate::codegen::reflection::ident(crate::protocol::constructor::ACTIVATE);
    zyn! {
        @match (*mode) {
            ConstructorMode::Deferred => {
                if false {
                    {{ service.clone() }}::{{ activate.clone() }}({{ context.clone() }})
                } else {
                    @ConstructAutomaticInjectable(
                        analysis = analysis.clone(),
                        service = service.clone(),
                        context = context.clone(),
                    )
                }
            }
            ConstructorMode::Automatic => {
                @ConstructAutomaticInjectable(
                    analysis = analysis.clone(),
                    service = service.clone(),
                    context = context.clone(),
                )
            }
            ConstructorMode::Explicit => {
                {{ service.clone() }}::{{ activate.clone() }}({{ context.clone() }})
            }
        }
    }
}

#[zyn::element]
fn construct_automatic_injectable(
    analysis: AnalyzedFields,
    service: zyn::TokenStream,
    context: syn::Ident,
) -> zyn::TokenStream {
    // 输入和结果绑定共享 bridge 的定义处卫生；字段中的业务表达式保持原 token。
    let instance = syn::Ident::new("__nestrs_injectable_instance", context.span());
    zyn! {
        @if (analysis.has_injected_fields()) {
            // 重用带定义处卫生的输入绑定名，不增加逐参数临时量。
            // tuple 全部求值成功并通过消费检查后，才执行下面的业务字段表达式。
            let {{ context }} = (
                @for (spec in analysis.specs.iter().filter(|spec| spec.is_injected())) {
                    @TakeInjectedFieldValue(
                        spec = spec.clone(),
                        context = context.clone(),
                    ),
                }
                {
                    {{ context }}.ensure_all_consumed()?;
                    ::core::mem::drop({{ context }});
                },
            );
        } @else {
            {{ context }}.ensure_all_consumed()?;
            ::core::mem::drop({{ context }});
        }
        let {{ instance }} = @ConstructInjectableInstance(
            analysis = analysis.clone(),
            service = service.clone(),
            context = context.clone(),
        );
        ::core::result::Result::Ok(
            ::nestrs_core::activation::ErasedService::new({{ instance }})
        )
    }
}

/// 输出使用共享字段分析构造一个具体 service 的表达式。
///
/// `service` 对普通 component 是结构体标识符，对开放泛型 component 则是 `Self`；
/// 两种路径因此共享同一份字段策略、输入位置和 `#[value]` 语义。
#[zyn::element]
fn construct_injectable_instance(
    analysis: AnalyzedFields,
    service: zyn::TokenStream,
    context: syn::Ident,
) -> zyn::TokenStream {
    zyn! {
        @match (&analysis.item.fields) {
            Fields::Named(fields) => {
                {{ service }} {
                    @for (index in 0..fields.named.len()) {
                        {{ fields.named[index].ident.as_ref().expect("named field must have an identifier") }}:
                        @ConstructInjectableField(
                            field_type = fields.named[index].ty.clone(),
                            spec = analysis.specs[index].clone(),
                            context = context.clone(),
                        ),
                    }
                }
            }
            Fields::Unnamed(fields) => {
                {{ service }}(
                    @for (index in 0..fields.unnamed.len()) {
                        @ConstructInjectableField(
                            field_type = fields.unnamed[index].ty.clone(),
                            spec = analysis.specs[index].clone(),
                            context = context.clone(),
                        ),
                    }
                )
            }
            Fields::Unit => {
                {{ service }}
            }
        }
    }
}

/// 为一个结构体字段选择其构造表达式。
///
/// 字段保持在最终 struct literal 中，因而 `#[value(...)]` 的表达式仍在字段类型
/// 上下文中按声明顺序求值。注入字段从类型化 tuple 移出，不增加调用点可见的变量名。
#[zyn::element]
fn construct_injectable_field(
    field_type: syn::Type,
    spec: FieldSpec,
    context: syn::Ident,
) -> zyn::TokenStream {
    zyn! {
        @if (spec.is_injected()) {
            {{ context }}.{{ syn::Index::from(spec.input_slot.expect("injected field")) }}
        } @else {
            @RewriteValueField(
                field_type = field_type.clone(),
                strategy = spec.strategy.clone(),
            )
        }
    }
}

/// 从预绑定的 `ConstructionInputs` 取出一个注入字段。
#[zyn::element]
fn take_injected_field_value(spec: FieldSpec, context: syn::Ident) -> zyn::TokenStream {
    let FieldStrategy::Inject {
        service_type,
        optional,
        lazy,
        ..
    } = &spec.strategy
    else {
        unreachable!("only injected fields may consume a construction input")
    };
    let service_type = service_type.clone();
    let optional = *optional;
    let lazy = *lazy;
    let position = spec
        .input_slot
        .expect("inject field must have an input slot");

    zyn! {
        @if (lazy) {
            @if (optional) {
                {{ context }}.take_optional_lazy::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ position }})
                )?
            } @else {
                {{ context }}.take_lazy::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ position }})
                )?
            }
        } @else {
            @if (optional) {
                {{ context }}.take_optional::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ position }})
                )?
            } @else {
                {{ context }}.take::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ position }})
                )?
            }
        }
    }
}

fn context_binding(analysis: &AnalyzedFields, context: &syn::Ident) -> zyn::TokenStream {
    if analysis.has_injected_fields() {
        quote!(mut #context: ::nestrs_core::activation::ConstructionInputs)
    } else {
        quote!(#context: ::nestrs_core::activation::ConstructionInputs)
    }
}

fn context_identifier(item: &ItemStruct, binding_span: zyn::proc_macro2::Span) -> syn::Ident {
    // 业务类型继续使用原 Ident；只有拼接内部符号时移除 raw 前缀。
    let service = item.ident.unraw();
    syn::Ident::new(
        &format!("__nestrs_injectable_context_for_{service}"),
        binding_span.located_at(item.ident.span()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::injection::macros::injectable::field_analyze::{
        AnalyzedFields, collect_field_specs,
    };
    use zyn::{Render, syn};

    fn render_constructor(item: ItemStruct, specs: Vec<FieldSpec>) -> String {
        GenerateInjectableConstructor {
            binding_span: zyn::proc_macro2::Span::mixed_site(),
            analysis: AnalyzedFields { item, specs },
            mode: ConstructorMode::Deferred,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn constructor_modes_render_only_the_requested_candidates() {
        for generic in [false, true] {
            for mode in [
                ConstructorMode::Deferred,
                ConstructorMode::Automatic,
                ConstructorMode::Explicit,
            ] {
                let item: ItemStruct = if generic {
                    syn::parse_quote!(
                        struct Service<T> {
                            value: T,
                        }
                    )
                } else {
                    syn::parse_quote!(
                        struct Service {
                            value: NoDefault,
                        }
                    )
                };
                let specs = collect_field_specs(&item.fields).unwrap();
                let analysis = AnalyzedFields { item, specs };
                let input = zyn::Input::default();
                let rendered = if generic {
                    GenerateGenericInjectableConstructor {
                        analysis,
                        mode,
                        binding_span: zyn::proc_macro2::Span::mixed_site(),
                    }
                    .render(&input)
                } else {
                    GenerateInjectableConstructor {
                        analysis,
                        mode,
                        binding_span: zyn::proc_macro2::Span::mixed_site(),
                    }
                    .render(&input)
                };
                let tokens = rendered.tokens().to_string();
                assert_eq!(
                    tokens.contains("if false"),
                    mode == ConstructorMode::Deferred,
                    "{tokens}"
                );
                assert_eq!(
                    tokens.contains("__nestrs_constructor_activate"),
                    mode != ConstructorMode::Automatic,
                    "{tokens}"
                );
                assert_eq!(
                    tokens.contains("Default :: default"),
                    mode != ConstructorMode::Explicit,
                    "{tokens}"
                );
                assert_eq!(
                    tokens.contains("ensure_all_consumed"),
                    mode != ConstructorMode::Explicit,
                    "{tokens}"
                );
            }
        }
    }

    #[test]
    fn automatic_mode_preserves_business_conditionals_even_with_matching_helper_names() {
        let item: ItemStruct = syn::parse_quote! {
            struct Service {
                #[value(if false { Business::__nestrs_constructor_activate(()) } else { 3 })]
                value: usize,
            }
        };
        let specs = collect_field_specs(&item.fields).unwrap();
        let rendered = GenerateInjectableConstructor {
            binding_span: zyn::proc_macro2::Span::mixed_site(),
            analysis: AnalyzedFields { item, specs },
            mode: ConstructorMode::Automatic,
        }
        .render(&zyn::Input::default());
        assert!(
            rendered
                .tokens()
                .to_string()
                .contains("if false { Business :: __nestrs_constructor_activate (()) } else { 3 }")
        );
    }

    #[test]
    fn emits_a_context_backed_constructor_with_all_field_strategies() {
        let item: ItemStruct = syn::parse_str(
            r#"
            struct Consumer {
                #[inject]
                service: Service,
                #[value("label")]
                label: String,
                #[inject]
                audit: Option<dyn Audit>,
                enabled: bool,
            }
            "#,
        )
        .expect("test input should parse");
        let specs = collect_field_specs(&item.fields).expect("fields should be valid");
        let generated = render_constructor(item, specs);

        assert!(generated.contains("fn __nestrs_construct"));
        assert!(generated.contains("ConstructionInputs"));
        assert!(generated.contains("take :: < Service >"));
        assert!(generated.contains("InputSlot :: new (0usize)"));
        assert!(generated.contains("take_optional :: < dyn Audit >"));
        assert!(generated.contains("InputSlot :: new (1usize)"));
        assert!(generated.contains("Into :: < String > :: into (\"label\")"));
        assert!(generated.contains("Default :: default ()"));
        assert!(generated.contains("let __nestrs_injectable_instance = Consumer"));
        assert!(generated.contains("ErasedService :: new (__nestrs_injectable_instance)"));
        let construct = generated
            .find("let __nestrs_injectable_instance")
            .expect("the class instance should be built into a local first");
        let ensure = generated
            .find("ensure_all_consumed ()")
            .expect("the adapter should validate its input coverage");
        let erase = generated
            .find("ErasedService :: new (__nestrs_injectable_instance)")
            .expect("the validated instance should be erased last");
        let last_input = generated
            .find("take_optional :: < dyn Audit >")
            .expect("all typed inputs should be acquired first");
        let release_inputs = generated
            .find("mem :: drop")
            .expect("the consumed input buffer must be released before business construction");
        assert!(last_input < ensure);
        assert!(ensure < release_inputs && release_inputs < construct && construct < erase);
        assert!(!generated.contains("unsafe"));
    }

    #[test]
    fn raw_service_names_only_unraw_the_generated_context() {
        let item: ItemStruct = syn::parse_quote!(
            struct r#type;
        );
        let specs = collect_field_specs(&item.fields).unwrap();
        let generated = render_constructor(item, specs);
        assert!(generated.contains("__nestrs_injectable_context_for_type"));
        assert!(generated.contains("let __nestrs_injectable_instance = r#type"));
    }

    #[test]
    fn supports_tuple_and_unit_struct_construction() {
        let tuple: ItemStruct =
            syn::parse_str("struct Tuple(#[inject] Service, #[value(1)] usize);")
                .expect("tuple input should parse");
        let tuple_specs = collect_field_specs(&tuple.fields).expect("fields should be valid");
        let tuple_output = render_constructor(tuple, tuple_specs);
        assert!(tuple_output.contains("let __nestrs_injectable_instance = Tuple"));
        assert!(tuple_output.contains("take :: < Service >"));

        let unit: ItemStruct = syn::parse_str("struct Unit;").expect("unit input should parse");
        let unit_specs = collect_field_specs(&unit.fields).expect("fields should be valid");
        let unit_output = render_constructor(unit, unit_specs);
        assert!(unit_output.contains(
            "__nestrs_injectable_context_for_Unit : :: nestrs_core :: activation :: ConstructionInputs"
        ));
        assert!(unit_output.contains("let __nestrs_injectable_instance = Unit"));
    }
}
