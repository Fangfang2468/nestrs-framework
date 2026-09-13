//! 宏期依赖事实到 core 注册 ABI 的唯一渲染入口。
//!
//! `#[injectable]` 与 `#[factory]` 生成的 provider 载荷不同，但依赖描述、服务 key、
//! 生命周期与 cleanup hook 的表达式完全相同。这里集中渲染，两个入口只负责提供
//! 各自的宏期事实，避免同一份 ABI 出现两套实现。

use crate::injection::{
    attrs::{cleanup::CleanupPath, lifetime::ServiceLifetime, service_key::ServiceKey},
    request::{DependencyRequest, DependencyShape, classify},
};
use zyn::{syn, zyn};

/// 渲染一条依赖请求的注册描述。
///
/// 依赖的服务类型形态在这里一次性分流：concrete 与闭合泛型使用单个 monomorphized
/// preparer，trait object 的 projector 则必须由匹配到的 `#[bind]` 提供。
#[zyn::element]
pub(crate) fn emit_dependency_request(request: DependencyRequest) -> zyn::TokenStream {
    let service_type = request.service_type.clone();
    let shape = classify(&service_type);
    let is_concrete = shape != DependencyShape::TraitObject;
    let is_trait_object = shape == DependencyShape::TraitObject;
    let has_closed_provider = shape == DependencyShape::ClosedGeneric;
    let key = request.key.clone();
    let optional = request.optional;
    let declaration_position = request.declaration_position;
    let input_position = request.input_position;
    let label = request.label.clone();

    zyn! {
        ::nestrs_core::__private::InjectionSpec {
            declaration_position: {{ declaration_position }},
            input_position: ::nestrs_core::__private::InputPosition({{ input_position }}),
            label: @RenderFieldLabel(label = label.clone()),
            token: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
                @RenderServiceKey(key = key.clone()),
                ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type.clone() }}>(),
            ),
            optional: {{ optional }},
            target: @RenderDependencyTarget(
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            prepare_input: @RenderDependencyPreparer(
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

/// 为 concrete 类型渲染 Arena 输入准备函数项。
///
/// 函数项保留宏展开时已知的精确 `T`，运行时只需把查到的稳定地址传入；对 `dyn Trait`
/// 则暂不生成错误的薄指针转换，等待 `#[bind]` 提供 concrete-to-trait projector。
#[zyn::element]
fn render_dependency_preparer(
    service_type: syn::Type,
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

/// 将宏期字段类型的形状写入 runtime 注册 ABI。
///
/// 这里由 `syn::Type` 直接给出类别，而不是让 runtime 通过 `TypeId` 反推 `dyn Trait`。
/// 后者无法恢复 trait-object 的 vtable，也会把 unsupported 类型误判成 concrete 服务。
#[zyn::element]
fn render_dependency_target(is_concrete: bool, is_trait_object: bool) -> zyn::TokenStream {
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

/// 为一个已闭合的泛型服务请求渲染其具体化 callback。
///
/// `TypeId` 不能还原开放泛型的 origin 或实参；这个 callback 则在宏展开时已带着
/// `Repository<UserEntity>` 这样的精确类型，运行时只需在缺少显式注册时调用它。
/// 动态 trait 注入不会到达这里的 `Some` 分支，从而仍由 bind 的普通选择规则处理。
#[zyn::element]
fn render_closed_provider_callback(
    service_type: syn::Type,
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
