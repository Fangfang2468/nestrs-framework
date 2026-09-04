//! `#[injectable]` 的 linkme 反射元数据收集。
//!
//! 此模块只消费字段分析结果，不参与字段改写或构造代码生成。这样 provider 身份、
//! 字段 key、可选性和 DI 输入位置都来自与 `RewriteInjectionField` 相同的
//! [`FieldSpec`]，不会重新解析已被清除的 marker。

use crate::injection::attrs::{lifetime::ServiceLifetime, service_key::ServiceKey};

use super::{
    config::InjectableConfig,
    field_analyze::{AnalyzedFields, FieldSpec, FieldStrategy, is_generic_concrete_type_path},
};
use zyn::{
    quote::quote,
    syn::{self, Type},
    zyn,
};

/// 向 `REFLECT_METADATA_INJECTABLE` 写入一个组件元数据工厂。
///
/// 此 element 只生成 linkme 注册函数，不负责构造 adapter 或匿名作用域。调用方
/// 必须将它和 `GenerateInjectableConstructor` 放在同一个匿名 `const` 中，才能把
/// 词法私有的 `__nestrs_construct` 函数指针写入 metadata。
#[zyn::element]
pub(crate) fn collect_injectable_metadata(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
) -> zyn::TokenStream {
    let service = analysis.item.ident.clone();
    let service_type = quote!(#service);

    zyn! {
        #[::nestrs_core::__private::linkme::distributed_slice(
            ::nestrs_core::__private::REFLECT_METADATA_INJECTABLE
        )]
        #[linkme(crate = ::nestrs_core::__private::linkme)]
        fn __nestrs_reflect_metadata_injectable()
            -> ::nestrs_core::__private::StructComponent
        {
            ::nestrs_core::__private::StructComponent {
                @EmitInjectableComponentFields(
                    analysis = analysis.clone(),
                    config = config.clone(),
                    primary = *primary,
                    service_type = service_type.clone(),
                )
                constructor: __nestrs_construct,
            }
        }
    }
}

/// 输出一个 component 在 provider 身份、字段依赖和来源方面的公共元数据字段。
///
/// 常规闭合服务把这些字段包在 linkme factory 中；开放泛型服务则由
/// `ComponentDefinition::component()` 使用完全相同的字段。构造 adapter 有不同的
/// 可见性需求，故由调用方在这个 element 的输出之后单独提供 `constructor` 字段。
#[zyn::element]
pub(crate) fn emit_injectable_component_fields(
    analysis: AnalyzedFields,
    config: InjectableConfig,
    primary: bool,
    service_type: zyn::TokenStream,
) -> zyn::TokenStream {
    let provider_key = config.key.clone();
    let lifetime = config.lifetime;
    zyn! {
        service_identifier: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
            @RenderServiceKey(key = provider_key.clone()),
            ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type }}>(),
        ),
        lifetime: @RenderServiceLifetime(lifetime = lifetime),
        field_injections: ::std::vec![
            @for (spec in analysis.specs.iter()) {
                @if (spec.is_injected()) {
                    @EmitInjectableFieldMetadata(
                        spec = spec.clone(),
                    ),
                }
            }
        ],
        primary: {{ primary }},
        source: ::nestrs_core::registration::service_source::ServiceSource::new(
            file!(),
            line!(),
            column!(),
        ),
    }
}

/// 输出一条字段注入反射元数据。
///
/// `CollectInjectableMetadata` 已通过 `FieldSpec::is_injected` 过滤；这里保留
/// defensive check，因此以后单独复用这个 element 时也不会悄然生成伪 metadata。
#[zyn::element]
fn emit_injectable_field_metadata(spec: FieldSpec) -> zyn::TokenStream {
    let FieldStrategy::Inject {
        service_type,
        key,
        optional,
    } = &spec.strategy
    else {
        unreachable!("only injected fields have FieldInjection metadata")
    };

    let field_index = spec.index;
    let dependency_position = spec
        .dependency_position
        .expect("inject field must have a dependency position");
    let field_name = spec.field_name.clone();
    let service_type = service_type.clone();
    let key = key.clone();
    let optional = *optional;
    let has_component_definition = is_generic_concrete_type_path(&service_type);
    let is_concrete = matches!(service_type, Type::Path(_));
    let is_trait_object = matches!(service_type, Type::TraitObject(_));

    zyn! {
        ::nestrs_core::__private::FieldInjection {
            field_index: {{ field_index }},
            field_name: @RenderFieldName(field_name = field_name.clone()),
            dependency_position: {{ dependency_position }},
            service_identifier: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
                @RenderServiceKey(key = key.clone()),
                ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type }}>(),
            ),
            target: @RenderFieldInjectionTarget(
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            component_definition: @RenderComponentDefinitionCallback(
                service_type = service_type.clone(),
                has_component_definition = has_component_definition,
            ),
            prepare_input: @RenderArenaInputPreparer(
                service_type = service_type.clone(),
                optional = optional,
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            optional: {{ optional }},
        }
    }
}

/// 为一个已闭合的泛型服务请求渲染其具体化 callback。
///
/// `TypeId` 不能还原开放泛型的 origin 或实参；这个 callback 则在宏展开时已带着
/// `Repository<UserEntity>` 这样的精确类型，运行时只需在缺少显式注册时调用它。
/// 动态 trait 注入不会到达这里的 `Some` 分支，从而仍由 bind/primary 的普通解析
/// 规则处理。
#[zyn::element]
fn render_component_definition_callback(
    service_type: Type,
    has_component_definition: bool,
) -> zyn::TokenStream {
    zyn! {
        @if (*has_component_definition) {
            ::core::option::Option::Some(
                ::nestrs_core::__private::component_definition::<{{ service_type }}>
                    as ::nestrs_core::__private::ComponentDefinitionCallback
            )
        } @else {
            ::core::option::Option::None
        }
    }
}

/// 为 concrete 类型字段渲染 Arena 输入准备函数项。
///
/// 函数项保留宏展开时已知的精确 `T`，运行时只需把查到的稳定地址传入；对 `dyn Trait`
/// 则暂不生成错误的薄指针转换，等待 `#[bind]` 提供 concrete-to-trait projector。
#[zyn::element]
fn render_arena_input_preparer(
    service_type: Type,
    optional: bool,
    is_concrete: bool,
    is_trait_object: bool,
) -> zyn::TokenStream {
    zyn! {
        @if (*is_concrete) {
            ::core::option::Option::Some(
                @if (*optional) {
                    ::nestrs_core::__private::prepare_optional::<{{ service_type }}>
                } @else {
                    ::nestrs_core::__private::prepare_required::<{{ service_type }}>
                }
                as ::nestrs_core::__private::PrepareInput
            )
        } @else if (*is_trait_object && *optional) {
            ::core::option::Option::Some(
                ::nestrs_core::__private::prepare_optional_absent::<{{ service_type }}>
                    as ::nestrs_core::__private::PrepareInput
            )
        } @else {
            ::core::option::Option::None
        }
    }
}

/// 将宏期字段类型形态写入 runtime metadata。
///
/// 这里由 `syn::Type` 直接给出类别，而不是让 runtime 通过 `TypeId` 反推 `dyn Trait`。
/// 后者无法恢复 trait-object 的 vtable，也会把 unsupported 类型误判成 concrete 服务。
#[zyn::element]
fn render_field_injection_target(is_concrete: bool, is_trait_object: bool) -> zyn::TokenStream {
    zyn! {
        @if (*is_concrete) {
            ::nestrs_core::__private::FieldInjectionTarget::Concrete
        } @else if (*is_trait_object) {
            ::nestrs_core::__private::FieldInjectionTarget::TraitObject
        } @else {
            ::nestrs_core::__private::FieldInjectionTarget::Unsupported
        }
    }
}

/// 将宏期 `ServiceKey` 渲染为 core 的运行时 key 表达式。
#[zyn::element]
fn render_service_key(key: Option<ServiceKey>) -> zyn::TokenStream {
    zyn! {
        @match (key.as_ref()) {
            Some(ServiceKey::Named(name)) => {
                ::core::option::Option::Some(
                    ::nestrs_core::registration::service_key::ServiceKey::Named({{ name }})
                )
            }
            Some(ServiceKey::Indexed(index)) => {
                ::core::option::Option::Some(
                    ::nestrs_core::registration::service_key::ServiceKey::Indexed({{ index }})
                )
            }
            None => {
                ::core::option::Option::None
            }
        }
    }
}

/// 将宏期 lifetime 渲染为 core 的运行时 lifetime 表达式。
#[zyn::element]
fn render_service_lifetime(lifetime: ServiceLifetime) -> zyn::TokenStream {
    zyn! {
        @match (*lifetime) {
            ServiceLifetime::Singleton => {
                ::nestrs_core::lifetime::Lifetime::Singleton
            }
            ServiceLifetime::Scoped => {
                ::nestrs_core::lifetime::Lifetime::Scoped
            }
            ServiceLifetime::Transient => {
                ::nestrs_core::lifetime::Lifetime::Transient
            }
        }
    }
}

/// 将可选字段名渲染为反射 metadata 需要的静态字符串。
#[zyn::element]
fn render_field_name(field_name: Option<syn::Ident>) -> zyn::TokenStream {
    zyn! {
        @match (field_name.as_ref()) {
            Some(name) => {
                ::core::option::Option::Some(stringify!({{ name }}))
            }
            None => {
                ::core::option::Option::None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::injectable::field_analyze::{AnalyzedFields, collect_field_specs};
    use zyn::{Render, syn};

    fn render_metadata(
        item: syn::ItemStruct,
        specs: Vec<FieldSpec>,
        config: InjectableConfig,
        primary: bool,
    ) -> String {
        CollectInjectableMetadata {
            analysis: AnalyzedFields { item, specs },
            config,
            primary,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn emits_provider_and_field_metadata_from_the_shared_analysis() {
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

        let output = render_metadata(item, specs, config, true);

        assert!(output.contains("REFLECT_METADATA_INJECTABLE"));
        assert!(output.contains("constructor : __nestrs_construct"));
        assert!(output.contains("Lifetime :: Scoped"));
        assert!(output.contains("ServiceKey :: Named (\"controller\")"));
        assert!(output.contains("field_index : 0usize"));
        assert!(
            output.contains(
                "field_name : :: core :: option :: Option :: Some (stringify ! (database))"
            )
        );
        assert!(output.contains("dependency_position : 0usize"));
        assert!(output.contains("field_index : 2usize"));
        assert!(output.contains("dependency_position : 1usize"));
        assert!(output.contains("ServiceKey :: Indexed (7usize)"));
        assert!(!output.contains("field_index : 1usize"));
        assert!(output.contains("primary : true"));
    }
}
