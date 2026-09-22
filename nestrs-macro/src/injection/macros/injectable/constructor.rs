//! `#[injectable]` 隐藏构造 adapter 的生成。
//!
//! 此处只定义构造函数本身。它必须由 `registration` 放入与 linkme provider
//! factory 相同的匿名 `const` 作用域，才能把函数指针写入 `Provider::Class`，同时
//! 不把 helper 暴露为结构体的 inherent method。

use super::{
    field_analyze::{AnalyzedFields, FieldSpec, FieldStrategy},
    field_initialization::RewriteValueField,
};
use zyn::{
    quote::quote,
    syn::{self, Fields, ItemStruct},
    zyn,
};

/// 输出一个仅供同一匿名注册作用域使用的构造 adapter。
///
/// 该 element 不自行添加 `const` 包裹。若作为顶层 sibling 输出，provider 就无法
/// 词法引用 `__nestrs_construct`；因此只能由 `EmitInjectableRegistration` 嵌入。
#[zyn::element]
pub(crate) fn generate_injectable_constructor(analysis: AnalyzedFields) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let context = context_identifier(&analysis.item);
    let context_binding = context_binding(&analysis, &context);
    let service = quote!(#service);

    zyn! {
        fn __nestrs_construct(
            {{ context_binding }}
        ) -> ::core::result::Result<
            ::nestrs_core::__private::ErasedService,
            ::nestrs_core::__private::ConstructionError,
        > {
            let __nestrs_injectable_instance = @ConstructInjectableInstance(
                analysis = analysis.clone(),
                service = service.clone(),
                context = context.clone(),
            );
            {{ context }}.ensure_all_consumed()?;
            ::core::result::Result::Ok(
                ::nestrs_core::__private::ErasedService::new(
                    __nestrs_injectable_instance
                )
            )
        }
    }
}

/// 输出开放泛型 `ProviderDefinition` 使用的无捕获构造 closure。
///
/// 它位于 trait 方法内部，因此 `Self` 已是由注入点单态化的服务类型；不像闭合
/// component 的 linkme 注册，这里绝不能生成一个全局命名函数或把开放 provider
/// 放进 distributed slice。
#[zyn::element]
pub(crate) fn generate_generic_injectable_constructor(
    analysis: AnalyzedFields,
) -> zyn::TokenStream {
    let context = context_identifier(&analysis.item);
    let context_binding = context_binding(&analysis, &context);
    let service = quote!(Self);

    zyn! {
        |{{ context_binding }}| -> ::core::result::Result<
            ::nestrs_core::__private::ErasedService,
            ::nestrs_core::__private::ConstructionError,
        > {
            let __nestrs_injectable_instance = @ConstructInjectableInstance(
                analysis = analysis.clone(),
                service = service.clone(),
                context = context.clone(),
            );
            {{ context }}.ensure_all_consumed()?;
            ::core::result::Result::Ok(
                ::nestrs_core::__private::ErasedService::new(
                    __nestrs_injectable_instance
                )
            )
        }
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
/// 上下文中按声明顺序求值，也不会引入可遮蔽调用点名称的临时变量。
#[zyn::element]
fn construct_injectable_field(
    field_type: syn::Type,
    spec: FieldSpec,
    context: syn::Ident,
) -> zyn::TokenStream {
    zyn! {
        @if (spec.is_injected()) {
            @TakeInjectedFieldValue(
                spec = spec.clone(),
                context = context.clone(),
            )
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
        ..
    } = &spec.strategy
    else {
        unreachable!("only injected fields may consume a construction input")
    };
    let service_type = service_type.clone();
    let optional = *optional;
    let position = spec
        .input_slot
        .expect("inject field must have an input slot");

    zyn! {
        @if (optional) {
            {{ context }}.take_optional::<{{ service_type }}>(
                ::nestrs_core::__private::InputSlot::new({{ position }})
            )?
        } @else {
            {{ context }}.take::<{{ service_type }}>(
                ::nestrs_core::__private::InputSlot::new({{ position }})
            )?
        }
    }
}

fn context_binding(analysis: &AnalyzedFields, context: &syn::Ident) -> zyn::TokenStream {
    if analysis.has_injected_fields() {
        quote!(mut #context: ::nestrs_core::__private::ConstructionInputs)
    } else {
        quote!(#context: ::nestrs_core::__private::ConstructionInputs)
    }
}

fn context_identifier(item: &ItemStruct) -> syn::Ident {
    let service = &item.ident;
    zyn::format_ident!("__nestrs_injectable_context_for_{service}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::macros::injectable::field_analyze::{
        AnalyzedFields, collect_field_specs,
    };
    use zyn::{Render, syn};

    fn render_constructor(item: ItemStruct, specs: Vec<FieldSpec>) -> String {
        GenerateInjectableConstructor {
            analysis: AnalyzedFields { item, specs },
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
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
        assert!(construct < ensure && ensure < erase);
        assert!(!generated.contains("unsafe"));
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
            "__nestrs_injectable_context_for_Unit : :: nestrs_core :: __private :: ConstructionInputs"
        ));
        assert!(unit_output.contains("let __nestrs_injectable_instance = Unit"));
    }
}
