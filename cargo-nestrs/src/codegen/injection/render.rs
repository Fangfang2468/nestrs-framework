//! 宏期依赖事实到 core 注册 ABI 的唯一渲染入口。
//!
//! `#[injectable]` 与 `#[factory]` 生成的 provider 载荷不同，但依赖描述、服务 key、
//! 生命周期与 cleanup hook 的表达式完全相同。这里集中渲染，两个入口只负责提供
//! 各自的宏期事实，避免同一份 ABI 出现两套实现。

use crate::codegen::injection::{
    macros_attrs::{cleanup::CleanupPath, lifetime::ServiceLifetime, service_key::ServiceKeySpec},
    sub_macros::inject::{DependencyRequest, is_trait_object},
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
    let is_trait_object = is_trait_object(&service_type);
    let key = request.key.clone();
    let optional = request.optional;
    let declaration_position = request.declaration_position;
    let input_slot = request.input_slot;
    let label = request.label.clone();

    zyn! {
        ::nestrs_core::registration::dependency::DependencyRequest {
            declaration_position: {{ declaration_position }},
            input_slot: ::nestrs_core::activation::InputSlot::new({{ input_slot }}),
            token: ::nestrs_core::service::ServiceIdentifier::new(
                @RenderServiceKey(key = key.clone()),
                {
                    ::nestrs_core::registration::compiler::compiler_dependency::<{{ service_type.clone() }}, {{ input_slot }}>();
                    ::nestrs_core::service::ServiceType::create::<{{ service_type.clone() }}>()
                },
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
            ),
        }
    }
}

/// 渲染依赖值准备为构造输入槽位载荷的方式。
///
/// 已知 dyn 写法使用 binding；其余类型路径可能是别名或泛型参数，由冻结后的实际
/// 路由选择准确的 concrete preparer 或 binding。typed address 检查支持 ?Sized，
/// 不需要从源码拼写猜测一个路径是否代表 trait object。
#[zyn::element]
fn render_delivery(
    service_type: syn::Type,
    optional: bool,
    is_trait_object: bool,
) -> zyn::TokenStream {
    zyn! {
        @if (*is_trait_object) {
            @if (*optional) {
                ::nestrs_core::registration::dependency::Delivery::RequiresBindingOrAbsent(
                    ::nestrs_core::activation::prepare_optional_absent::<{{ service_type }}>
                        as ::nestrs_core::activation::InputPreparer
                )
            } @else {
                ::nestrs_core::registration::dependency::Delivery::RequiresBinding
            }
        } @else {
            ::nestrs_core::registration::dependency::Delivery::Selected(
                @if (*optional) {
                    ::nestrs_core::activation::prepare_optional::<{{ service_type }}>
                } @else {
                    ::nestrs_core::activation::prepare_required::<{{ service_type }}>
                }
                as ::nestrs_core::activation::InputPreparer
            )
        }
    }
}

/// 渲染解析期寻找 provider 的方式。
///
/// 具体类型处探测可选蓝图，factory-only 服务无需 ProviderDefinition 约束。
/// 泛型体中的探测不重新特化；driver 补充真实闭合类型和类型安全依赖路径的被动
/// 蓝图目录，graph 在显式注册缺席时读取它，冻结后不再访问目录。
#[zyn::element]
fn render_provider_source(service_type: syn::Type) -> zyn::TokenStream {
    zyn! {
        {
            use ::nestrs_core::registration::root::ProbeProvider as _;
            let probe = ::nestrs_core::registration::root::Probe::<{{ service_type }}>::new();
            #[allow(clippy::needless_borrow)]
            match (&&probe).provider_callback() {
                Some(callback) => ::nestrs_core::registration::dependency::ProviderSource::Materialize(callback),
                None => ::nestrs_core::registration::dependency::ProviderSource::Registered,
            }
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

/// 将宏期 `ServiceKeySpec` 渲染为 core 的唯一运行时 key 表达式。
#[zyn::element]
pub(crate) fn render_service_key(key: Option<ServiceKeySpec>) -> zyn::TokenStream {
    zyn! {
        @match (key.as_ref()) {
            Some(ServiceKeySpec::Named(name)) => {
                ::core::option::Option::Some(
                    ::nestrs_core::ServiceKey::Named(
                        ::std::string::String::from({{ name }})
                    )
                )
            }
            Some(ServiceKeySpec::Indexed(index)) => {
                ::core::option::Option::Some(
                    ::nestrs_core::ServiceKey::Indexed({{ index }})
                )
            }
            None => {
                ::core::option::Option::None
            }
        }
    }
}

/// 将同一个静态 key 直接写入编译器 marker，供语义发现按精确身份选择蓝图。
#[zyn::element]
pub(crate) fn emit_compiler_key(key: Option<ServiceKeySpec>) -> zyn::TokenStream {
    zyn! {
        @match (key.as_ref()) {
            Some(ServiceKeySpec::Named(name)) => {
                ::nestrs_core::registration::compiler::CompilerKey::Named({{ name }})
            }
            Some(ServiceKeySpec::Indexed(index)) => {
                ::nestrs_core::registration::compiler::CompilerKey::Indexed({{ index }})
            }
            None => {
                ::nestrs_core::registration::compiler::CompilerKey::Default
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
                ::nestrs_core::ServiceLifetime::Singleton
            }
            ServiceLifetime::Scoped => {
                ::nestrs_core::ServiceLifetime::Scoped
            }
            ServiceLifetime::Transient => {
                ::nestrs_core::ServiceLifetime::Transient
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
            (|| -> ::nestrs_core::registration::provider::CleanupFuture {
                ::std::boxed::Box::pin({{ cleanup_path }}())
            }) as ::nestrs_core::registration::provider::CleanupHook
        )
    }
}
