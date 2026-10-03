//! 开放泛型 `#[injectable]` 的 provider definition 生成。
//!
//! 最终执行计划只包含已经闭合的服务。`Repository<T>` 在自己的匿名作用域中
//! 生成局部 ProviderDefinition；driver 从查询或依赖取得真实闭合 Ty 后求解这个
//! 实现，再分析其 MIR。运行时输入不携带用于寻找或物化泛型的回调。

use super::{
    config::InjectableConfig,
    constructor::GenerateGenericInjectableConstructor,
    field_analyze::{AnalyzedFields, FieldStrategy},
    provider::EmitClassProviderFields,
};
use crate::codegen::constructor::ConstructorMode;
use crate::codegen::injection::render::{EmitCompilerKey, EmitPlanProvider};
use crate::{
    codegen::reflection,
    protocol::{self, Marker},
};
use zyn::{quote::quote, syn, zyn};

/// 为一个开放泛型 provider 输出其按需具体化的 provider definition。
///
/// 构造 adapter 是 trait 方法内的无捕获 closure，而不是可见的 inherent helper 或
/// 公开注册函数。实例化后的 `Self` 代表例如 `Repository<UserEntity>` 的闭合类型，
/// 类型身份和构造结果都继续由标准 Rust 保证；策略标记只供编译器读取。
#[zyn::element]
pub(crate) fn define_generic_injectable_provider(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
    lazy: Option<bool>,
    mode: ConstructorMode,
) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let provider_definition_generics = provider_definition_generics(analysis);
    let (impl_generics, type_generics, where_clause) =
        provider_definition_generics.split_for_impl();
    let service_type = quote!(Self);
    let reflection = reflection::support(true);
    let reflection_module = reflection::ident(protocol::REFLECTION_MODULE);
    let provider_definition = reflection::ident(protocol::PROVIDER_DEFINITION);
    let provider_marker = reflection::ident(Marker::Provider.name());

    zyn! {
        #[allow(clippy::unused_unit)]
        const _: () = {
            {{ reflection }}
            impl {{ impl_generics }} {{ reflection_module.clone() }}::{{ provider_definition }}
                for {{ service }} {{ type_generics }} {{ where_clause }}
            {
                fn provider() -> ::nestrs_core::activation::adapter::ActivationAdapter {
                    {{ reflection_module }}::{{ provider_marker }}::<Self>(
                        @EmitCompilerKey(key = config.key.clone())
                    );
                    @EmitPlanProvider(
                        service_type = service_type.clone(),
                        key = config.key.clone(),
                        lifetime = config.lifetime,
                        primary = *primary,
                        lazy = *lazy,
                    )
                    ::nestrs_core::activation::adapter::ActivationAdapter {
                        @EmitClassProviderFields(
                            analysis = analysis.clone(),
                            config = config.clone(),
                            service_type = service_type.clone(),
                            mode = *mode,
                        )
                        constructor: ::nestrs_core::activation::adapter::Constructor::Class(@GenerateGenericInjectableConstructor(
                            analysis = analysis.clone(),
                            mode = *mode,
                        )),
                    }
                }
            }
            ()
        };
    }
}

/// 保留原有泛型声明并额外约束闭合 `Self` 必须满足 injectable ABI。
///
/// 不能把 `Send + Sync + 'static` 机械附加到每个类型参数：服务可能通过 wrapper
/// 或自己的 where clause 满足这些条件。约束完整 self type 才能让泛型定义在声明
/// 处保持通用，同时只为真正可注入的闭合实例实现 `ProviderDefinition`。
fn provider_definition_generics(analysis: &AnalyzedFields) -> syn::Generics {
    let item = &analysis.item;
    let service = item.ident.clone();
    let (_, type_generics, _) = item.generics.split_for_impl();
    let self_type: syn::Type = syn::parse_quote!(#service #type_generics);
    let mut generics = item.generics.clone();
    let where_clause = generics.make_where_clause();
    where_clause.predicates.push(syn::parse_quote!(
        #self_type: ::nestrs_core::service::Injectable
    ));

    for spec in &analysis.specs {
        let FieldStrategy::Inject { service_type, .. } = &spec.strategy else {
            continue;
        };

        where_clause.predicates.push(syn::parse_quote!(
            #service_type: ::nestrs_core::service::Injectable
        ));
    }

    generics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::injection::{
        macros::injectable::field_analyze::analyze_fields, macros_attrs::lifetime::ServiceLifetime,
    };
    use zyn::{Render, syn};

    fn render_definition(item: syn::ItemStruct) -> String {
        render_definition_in_mode(item, ConstructorMode::Deferred)
    }

    fn render_definition_in_mode(item: syn::ItemStruct, mode: ConstructorMode) -> String {
        let analysis = analyze_fields(item).expect("generic item should analyze");
        DefineGenericInjectableProvider {
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
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn generic_editor_selection_controls_both_inputs_and_construction() {
        for mode in [ConstructorMode::Automatic, ConstructorMode::Explicit] {
            let output = render_definition_in_mode(
                syn::parse_quote! {
                    struct Repository<Entity: Clone> where Entity: Send {
                        dependency: Dependency<Entity>,
                    }
                },
                mode,
            );
            syn::parse_str::<syn::ItemConst>(&output).unwrap();
            assert!(output.contains("Entity : Clone"));
            assert!(output.contains("Entity : Send"));
            assert!(output.contains("for Repository < Entity >"));
            assert!(!output.contains("if false"), "{output}");
            assert_eq!(
                output.contains("__nestrs_constructor_dependencies"),
                mode == ConstructorMode::Explicit,
                "{output}"
            );
            assert_eq!(
                output.contains("__nestrs_constructor_activate"),
                mode == ConstructorMode::Explicit,
                "{output}"
            );
            assert_eq!(
                output.contains("Default :: default"),
                mode == ConstructorMode::Automatic,
                "{output}"
            );
        }
    }

    #[test]
    fn emits_a_conditional_provider_definition_without_eager_registration() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Repository<Entity> {
                marker: std::marker::PhantomData<Entity>,
            }
            "#,
        )
        .expect("test input should parse");
        let output = render_definition(item);

        assert!(output.contains("impl < Entity > __nestrs_reflect :: ProviderDefinition"));
        assert!(output.contains("for Repository < Entity > where Repository < Entity > : :: nestrs_core :: service :: Injectable"));
        assert!(output.contains("ServiceType :: create :: < Self >"));
        assert!(
            output.contains("Constructor :: Class (| __nestrs_injectable_context_for_Repository")
        );
        assert!(output.contains("let __nestrs_injectable_instance = Self"));
        assert!(output.contains("ensure_all_consumed ()"));
        assert!(!output.contains("REFLECTED_PROVIDERS"));
        assert!(!output.contains("fn __nestrs_construct"));
    }

    #[test]
    fn preserves_the_provider_where_clause() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Repository<Entity>
            where
                Entity: Clone,
            {
                marker: std::marker::PhantomData<Entity>,
            }
            "#,
        )
        .expect("test input should parse");
        let output = render_definition(item);

        assert!(output.contains("Entity : Clone"));
        assert!(output.contains("Repository < Entity > : :: nestrs_core :: service :: Injectable"));
    }

    #[test]
    fn keeps_injected_field_inputs_inside_the_hidden_closure() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Repository<Entity> {
                #[inject]
                storage: Storage,
                marker: std::marker::PhantomData<Entity>,
            }
            "#,
        )
        .expect("test input should parse");
        let output = render_definition(item);

        assert!(output.contains("| mut __nestrs_injectable_context_for_Repository"));
        assert!(output.contains("take :: < Storage >"));
        assert!(output.contains("InputSlot :: new (0usize)"));
    }
}
