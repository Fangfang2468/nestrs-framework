//! 宏期依赖事实到 core 注册 ABI 的唯一渲染入口。
//!
//! `#[injectable]` 与 `#[factory]` 生成的 provider 载荷不同，但依赖描述、服务 key、
//! 生命周期与 cleanup hook 的表达式完全相同。这里集中渲染，两个入口只负责提供
//! 各自的宏期事实，避免同一份 ABI 出现两套实现。

use crate::injection::{
    macros_attrs::{cleanup::CleanupPath, lifetime::ServiceLifetime, service_key::ServiceKey},
    sub_macros::inject::{DependencyRequest, DependencyShape, classify},
};
use zyn::{syn, zyn};

/// 渲染一条依赖请求的注册描述。
///
/// 请求的两个正交事实在这里一次性分流：值怎么进槽位（`delivery`）与 provider 从哪来
/// （`provider_source`）。concrete 与闭合泛型的交付方式相同，差别只在 provider 需要
/// 显式注册还是按需物化；trait object 的 projector 则必须由匹配到的 `#[bind]` 提供。
#[zyn::element]
pub(crate) fn emit_dependency_request(request: DependencyRequest) -> zyn::TokenStream {
    let service_type = request.service_type.clone();
    let shape = classify(&service_type);
    let is_trait_object = shape == DependencyShape::TraitObject;
    let materializes = shape == DependencyShape::ClosedGeneric;
    let key = request.key.clone();
    let optional = request.optional;
    let declaration_position = request.declaration_position;
    let input_position = request.input_position;
    let label = request.label.clone();

    zyn! {
        ::nestrs_core::__private::DependencyRequest {
            declaration_position: {{ declaration_position }},
            input_position: ::nestrs_core::__private::InputPosition({{ input_position }}),
            token: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
                @RenderServiceKey(key = key.clone()),
                ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type.clone() }}>(),
            ),
            optional: {{ optional }},
            label: @RenderFieldLabel(label = label.clone()),
            delivery: @RenderDelivery(
                service_type = service_type.clone(),
                optional = optional,
                is_trait_object = is_trait_object,
            ),
            provider_source: @RenderProviderSource(
                service_type = service_type.clone(),
                materializes = materializes,
            ),
        }
    }
}

/// 渲染依赖值写入构造输入槽位的方式。
///
/// concrete 与闭合泛型都由消费点自己单态化 preparer；trait object 的 projector 只能
/// 由匹配到的 `#[bind]` 提供，因此必选形态不携带 preparer，可选形态只携带一个
/// 「只接受缺席」的兜底函数项。
#[zyn::element]
fn render_delivery(
    service_type: syn::Type,
    optional: bool,
    is_trait_object: bool,
) -> zyn::TokenStream {
    zyn! {
        @if (*is_trait_object) {
            @if (*optional) {
                ::nestrs_core::__private::Delivery::RequiresBindingOrAbsent(
                    ::nestrs_core::__private::prepare_optional_absent::<{{ service_type }}>
                        as ::nestrs_core::__private::PrepareInput
                )
            } @else {
                ::nestrs_core::__private::Delivery::RequiresBinding
            }
        } @else {
            ::nestrs_core::__private::Delivery::Direct(
                @if (*optional) {
                    ::nestrs_core::__private::prepare_optional::<{{ service_type }}>
                } @else {
                    ::nestrs_core::__private::prepare_required::<{{ service_type }}>
                }
                as ::nestrs_core::__private::PrepareInput
            )
        }
    }
}

/// 渲染解析期寻找 provider 的方式。
///
/// `TypeId` 不能还原开放泛型的 origin 或实参；闭合泛型因此在这里嵌入一个返回精确
/// provider 的 callback，运行时只在缺少显式注册时调用它。
#[zyn::element]
fn render_provider_source(service_type: syn::Type, materializes: bool) -> zyn::TokenStream {
    zyn! {
        @if (*materializes) {
            ::nestrs_core::__private::ProviderSource::Materialize(
                ::nestrs_core::__private::provider_definition::<{{ service_type }}>
                    as ::nestrs_core::__private::ClosedProviderCallback
            )
        } @else {
            ::nestrs_core::__private::ProviderSource::Registered
        }
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
pub(crate) fn render_service_lifetime(lifetime: ServiceLifetime) -> zyn::TokenStream {
    zyn! {
        @match (lifetime) {
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
pub(crate) fn render_cleanup_hook(cleanup: Option<CleanupPath>) -> zyn::TokenStream {
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
