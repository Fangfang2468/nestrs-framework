//! `#[injectable]` 的 `Provider` 生成。
//!
//! 此模块只消费字段分析结果，不参与字段改写或构造代码生成。这样 provider 身份、
//! 字段 key、可选性和 DI 输入位置都来自与 `RewriteInjectionField` 相同的
//! `FieldSpec`，不会重新解析已被清除的 marker。

use crate::injection::render::{
    EmitDependencyRequest, RenderCleanupHook, RenderServiceKey, RenderServiceLifetime,
};

use super::{config::InjectableConfig, field_analyze::AnalyzedFields};
use zyn::{quote::quote, zyn};

/// 向统一的 `REFLECTED_PROVIDERS` slice 写入一个 class provider 工厂。
///
/// 此 element 只生成 linkme 注册函数，不负责构造 adapter 或匿名作用域。调用方
/// 必须将它和 `GenerateInjectableConstructor` 放在同一个匿名 `const` 中，才能把
/// 词法私有的 `__nestrs_construct` 函数指针写入 provider。
#[zyn::element]
pub(crate) fn collect_injectable_provider(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let service_type = quote!(#service);

    zyn! {
        #[::nestrs_core::__private::linkme::distributed_slice(
            ::nestrs_core::__private::REFLECTED_PROVIDERS
        )]
        #[linkme(crate = ::nestrs_core::__private::linkme)]
        fn __nestrs_reflect_provider() -> ::nestrs_core::__private::Provider {
            ::nestrs_core::__private::Provider::Class(
                ::nestrs_core::__private::ClassProvider {
                    @EmitClassProviderFields(
                        analysis = analysis.clone(),
                        config = config.clone(),
                        primary = *primary,
                        service_type = service_type.clone(),
                    )
                    constructor: __nestrs_construct,
                }
            )
        }
    }
}

/// 输出一个 class provider 在身份、依赖与公共属性上的字段。
///
/// 常规闭合服务把这些字段包在 linkme factory 中；开放泛型服务则由
/// `ProviderDefinition::provider()` 使用完全相同的字段。构造 adapter 有不同的
/// 词法可见性需求，故由调用方在这个 element 的输出之后单独提供 `constructor`。
#[zyn::element]
pub(crate) fn emit_class_provider_fields(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
    service_type: zyn::TokenStream,
) -> zyn::TokenStream {
    let provider_key = config.key.clone();
    let lifetime = config.lifetime;
    let cleanup = config.cleanup.clone();

    zyn! {
        provide: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
            @RenderServiceKey(key = provider_key.clone()),
            ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type }}>(),
        ),
        common: ::nestrs_core::__private::ProviderCommon {
            lifetime: @RenderServiceLifetime(lifetime = lifetime),
            primary: {{ primary }},
            source: ::nestrs_core::registration::service_source::ServiceSource::new(
                file!(),
                line!(),
                column!(),
            ),
            cleanup: @RenderCleanupHook(cleanup = cleanup.clone()),
        },
        dependencies: ::std::vec![
            @for (spec in analysis.specs.iter()) {
                @if (spec.is_injected()) {
                    @EmitDependencyRequest(request = spec.dependency_request()),
                }
            }
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::{
        attrs::{lifetime::ServiceLifetime, service_key::ServiceKey},
        injectable::field_analyze::{AnalyzedFields, FieldSpec, collect_field_specs},
    };
    use zyn::{Render, syn};

    fn render_provider(
        item: syn::ItemStruct,
        specs: Vec<FieldSpec>,
        config: InjectableConfig,
        primary: bool,
    ) -> String {
        CollectInjectableProvider {
            analysis: AnalyzedFields { item, specs },
            config,
            primary,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
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
            key: Some(ServiceKey::Named("controller".to_owned())),
            cleanup: None,
        };

        let output = render_provider(item, specs, config, true);

        assert!(output.contains("REFLECTED_PROVIDERS"));
        assert!(output.contains("Provider :: Class"));
        assert!(output.contains("constructor : __nestrs_construct"));
        assert!(output.contains("Lifetime :: Scoped"));
        assert!(output.contains("ServiceKey :: Named (\"controller\")"));
        assert!(output.contains("declaration_position : 0usize"));
        assert!(output.contains("input_position : :: nestrs_core :: __private :: InputPosition (0usize)"));
        assert!(output.contains("declaration_position : 2usize"));
        assert!(output.contains("input_position : :: nestrs_core :: __private :: InputPosition (1usize)"));
        assert!(output.contains("ServiceKey :: Indexed (7usize)"));
        assert!(!output.contains("declaration_position : 1usize"));
        assert!(output.contains("primary : true"));
    }
}
