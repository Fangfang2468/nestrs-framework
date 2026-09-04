//! 开放泛型 `#[injectable]` 的 component definition 生成。
//!
//! linkme distributed slice 只能保存已经闭合的服务定义。`Repository<T>` 这类
//! provider 因而不注册进 slice；当另一个闭合 component 请求
//! `Repository<UserEntity>` 时，字段 metadata 中的 callback 才会单态化并调用这里
//! 生成的 [`::nestrs_core::__private::ComponentDefinition`] 实现。

use super::{
    config::InjectableConfig,
    constructor::GenerateGenericInjectableConstructor,
    field_analyze::{AnalyzedFields, FieldStrategy, is_generic_concrete_type_path},
    metadata::EmitInjectableComponentFields,
};
use zyn::{quote::quote, syn, zyn};

/// 为一个开放泛型 provider 输出其按需具体化的 component definition。
///
/// 构造 adapter 是 trait 方法内的无捕获 closure，而不是可见的 inherent helper 或
/// linkme 注册函数。`Self` 在这里已经代表例如 `Repository<UserEntity>` 的闭合类型，
/// 因此 `ServiceType`、构造结果和缓存 key 都继续使用现有的具体服务 ABI。
#[zyn::element]
pub(crate) fn define_generic_injectable_component(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let component_generics = component_definition_generics(&analysis);
    let (impl_generics, type_generics, where_clause) = component_generics.split_for_impl();
    let service_type = quote!(Self);

    zyn! {
        impl {{ impl_generics }} ::nestrs_core::__private::ComponentDefinition
            for {{ service }} {{ type_generics }} {{ where_clause }}
        {
            fn component() -> ::nestrs_core::__private::StructComponent {
                ::nestrs_core::__private::StructComponent {
                    @EmitInjectableComponentFields(
                        analysis = analysis.clone(),
                        config = config.clone(),
                        primary = *primary,
                        service_type = service_type.clone(),
                    )
                    constructor: @GenerateGenericInjectableConstructor(
                        analysis = analysis.clone(),
                    ),
                }
            }
        }
    }
}

/// 保留原有泛型声明并额外约束闭合 `Self` 必须满足 injectable ABI。
///
/// 不能把 `Send + Sync + 'static` 机械附加到每个类型参数：服务可能通过 wrapper
/// 或自己的 where clause 满足这些条件。约束完整 self type 才能让泛型定义在声明
/// 处保持通用，同时只为真正可注入的闭合实例实现 `ComponentDefinition`。
fn component_definition_generics(analysis: &AnalyzedFields) -> syn::Generics {
    let item = &analysis.item;
    let service = item.ident.clone();
    let (_, type_generics, _) = item.generics.split_for_impl();
    let self_type: syn::Type = syn::parse_quote!(#service #type_generics);
    let mut generics = item.generics.clone();
    let where_clause = generics.make_where_clause();
    where_clause.predicates.push(syn::parse_quote!(
        #self_type: ::nestrs_core::__private::Injectable
    ));

    for spec in &analysis.specs {
        let FieldStrategy::Inject { service_type, .. } = &spec.strategy else {
            continue;
        };

        where_clause.predicates.push(syn::parse_quote!(
            #service_type: ::nestrs_core::__private::Injectable
        ));

        if is_generic_concrete_type_path(service_type) {
            where_clause.predicates.push(syn::parse_quote!(
                #service_type: ::nestrs_core::__private::ComponentDefinition
            ));
        }
    }

    generics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::{
        attrs::lifetime::ServiceLifetime, injectable::field_analyze::analyze_fields,
    };
    use zyn::{Render, syn};

    fn render_definition(item: syn::ItemStruct) -> String {
        let analysis = analyze_fields(item).expect("generic item should analyze");
        DefineGenericInjectableComponent {
            analysis,
            config: InjectableConfig {
                lifetime: ServiceLifetime::Singleton,
                key: None,
                cleanup: None,
            },
            primary: false,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn emits_a_conditional_component_definition_without_linkme_registration() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Repository<Entity> {
                marker: std::marker::PhantomData<Entity>,
            }
            "#,
        )
        .expect("test input should parse");
        let output = render_definition(item);

        assert!(
            output.contains("impl < Entity > :: nestrs_core :: __private :: ComponentDefinition")
        );
        assert!(output.contains("for Repository < Entity > where Repository < Entity > : :: nestrs_core :: __private :: Injectable"));
        assert!(output.contains("ServiceType :: create :: < Self >"));
        assert!(output.contains("constructor : | _nestrs_injectable_context_for_Repository"));
        assert!(output.contains("ErasedService :: new (Self"));
        assert!(!output.contains("REFLECT_METADATA_INJECTABLE"));
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
        assert!(
            output.contains("Repository < Entity > : :: nestrs_core :: __private :: Injectable")
        );
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
        assert!(output.contains("InputPosition (0usize)"));
    }
}
