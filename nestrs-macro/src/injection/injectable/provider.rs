//! `#[injectable]` 的 `Provider` 生成。
//!
//! 此模块只消费字段分析结果，不参与字段改写或构造代码生成。这样 provider 身份、
//! 字段 key、可选性和 DI 输入位置都来自与 `RewriteInjectionField` 相同的
//! [`FieldSpec`]，不会重新解析已被清除的 marker。

use crate::injection::attrs::{
    cleanup::CleanupPath, lifetime::ServiceLifetime, service_key::ServiceKey,
};

use super::{
    config::InjectableConfig,
    field_analyze::{AnalyzedFields, FieldSpec, FieldStrategy, is_generic_concrete_type_path},
};
use zyn::{
    quote::quote,
    syn::{self, Type},
    zyn,
};

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
            ::nestrs_core::__private::Provider::Class {
                @EmitClassProviderFields(
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
                    @EmitInjectionSpec(spec = spec.clone()),
                }
            }
        ],
    }
}

/// 输出一条字段依赖描述。
///
/// `CollectInjectableProvider` 已通过 `FieldSpec::is_injected` 过滤；这里保留
/// defensive check，因此以后单独复用这个 element 时也不会悄然生成伪依赖。
#[zyn::element]
fn emit_injection_spec(spec: FieldSpec) -> zyn::TokenStream {
    let FieldStrategy::Inject {
        service_type,
        key,
        optional,
    } = &spec.strategy
    else {
        unreachable!("only injected fields have InjectionSpec data")
    };

    let declaration_position = spec.index;
    let input_position = spec
        .dependency_position
        .expect("inject field must have a dependency position");
    let label = spec.field_name.clone();
    let service_type = service_type.clone();
    let key = key.clone();
    let optional = *optional;
    let has_closed_provider = is_generic_concrete_type_path(&service_type);
    let is_concrete = matches!(service_type, Type::Path(_));
    let is_trait_object = matches!(service_type, Type::TraitObject(_));

    zyn! {
        ::nestrs_core::__private::InjectionSpec {
            declaration_position: {{ declaration_position }},
            input_position: ::nestrs_core::__private::InputPosition({{ input_position }}),
            label: @RenderFieldLabel(label = label.clone()),
            token: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
                @RenderServiceKey(key = key.clone()),
                ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type }}>(),
            ),
            optional: {{ optional }},
            target: @RenderInjectionTarget(
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            prepare_input: @RenderArenaInputPreparer(
                service_type = service_type.clone(),
                optional = optional,
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            closed_provider: @RenderClosedProviderCallback(
                service_type = service_type.clone(),
                has_closed_provider = has_closed_provider,
            ),
        }
    }
}

/// 为一个已闭合的泛型服务请求渲染其具体化 callback。
///
/// `TypeId` 不能还原开放泛型的 origin 或实参；这个 callback 则在宏展开时已带着
/// `Repository<UserEntity>` 这样的精确类型，运行时只需在缺少显式注册时调用它。
/// 动态 trait 注入不会到达这里的 `Some` 分支，从而仍由 bind 的普通选择规则处理。
#[zyn::element]
fn render_closed_provider_callback(
    service_type: Type,
    has_closed_provider: bool,
) -> zyn::TokenStream {
    zyn! {
        @if (*has_closed_provider) {
            ::core::option::Option::Some(
                ::nestrs_core::__private::provider_definition::<{{ service_type }}>
                    as ::nestrs_core::__private::ClosedProviderCallback
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

/// 将宏期字段类型形态写入 runtime provider ABI。
///
/// 这里由 `syn::Type` 直接给出类别，而不是让 runtime 通过 `TypeId` 反推 `dyn Trait`。
/// 后者无法恢复 trait-object 的 vtable，也会把 unsupported 类型误判成 concrete 服务。
#[zyn::element]
fn render_injection_target(is_concrete: bool, is_trait_object: bool) -> zyn::TokenStream {
    zyn! {
        @if (*is_concrete) {
            ::nestrs_core::__private::InjectionTarget::Concrete
        } @else if (*is_trait_object) {
            ::nestrs_core::__private::InjectionTarget::TraitObject
        } @else {
            ::nestrs_core::__private::InjectionTarget::Unsupported
        }
    }
}

/// 将宏期 `ServiceKey` 渲染为 core 的运行时 key 表达式。
#[zyn::element]
pub(crate) fn render_service_key(key: Option<ServiceKey>) -> zyn::TokenStream {
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

/// 生成 `ProviderCommon::cleanup` 所需的零参数 async hook adapter。
///
/// Rust 的 `async fn` 返回匿名 future，不能直接作为 `CleanupHook`。非捕获 closure
/// 在宏展开处把它装箱为统一的 `CleanupFuture`；函数路径不是零参数 async hook 时，
/// Rust 会在该 adapter 的 `Box::pin` 调用处报告类型错误。
#[zyn::element]
fn render_cleanup_hook(cleanup: Option<CleanupPath>) -> zyn::TokenStream {
    let Some(cleanup) = cleanup else {
        return zyn! {
            ::core::option::Option::None
        };
    };
    let cleanup_path = cleanup.func_path.clone();

    zyn! {
        ::core::option::Option::Some(
            (|| -> ::nestrs_core::__private::CleanupFuture {
                ::std::boxed::Box::pin({{ cleanup_path }}())
            }) as ::nestrs_core::__private::CleanupHook
        )
    }
}

/// 将可选字段名渲染为 provider 依赖描述所需的静态标签。
#[zyn::element]
fn render_field_label(label: Option<syn::Ident>) -> zyn::TokenStream {
    zyn! {
        @match (label.as_ref()) {
            Some(label) => {
                ::core::option::Option::Some(stringify!({{ label }}))
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
