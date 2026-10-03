//! 声明事实与类型化输入适配器的唯一渲染入口。
//!
//! `#[injectable]` 与 `#[factory]` 生成的 provider 载荷不同，但依赖描述、服务 key、
//! 生命周期标记与 cleanup hook 的表达式完全相同。这里集中渲染，两个入口只负责提供
//! 各自的宏期事实，避免同一份 ABI 出现两套实现。

use crate::codegen::injection::{
    macros_attrs::{
        cleanup::CleanupPath,
        lifetime::ServiceLifetime,
        service_key::{ServiceKeyKind, ServiceKeySpec},
    },
    sub_macros::inject::{DependencyRequest, is_trait_object},
};
use crate::{
    codegen::reflection::ident,
    protocol::{self, Initialization, Lifetime, Marker, OriginKind},
};
use zyn::{quote::quote_spanned, syn, syn::spanned::Spanned, zyn};

/// 一条输入仅保留类型化执行能力；key、槽位策略和标签供编译器读 MIR 后消解。
#[zyn::element]
pub(crate) fn emit_dependency_request(request: DependencyRequest) -> zyn::TokenStream {
    let service_type = request.service_type.clone();
    let is_trait_object = is_trait_object(&service_type);
    let key = request.key.clone();
    let optional = request.optional;
    let lazy = request.lazy;
    let input_slot = request.input_slot;
    let span = service_type.span();
    let label = syn::LitStr::new(
        &request
            .label
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_default(),
        request.label.as_ref().map(syn::Ident::span).unwrap_or(span),
    );
    let reflection = ident(protocol::REFLECTION_MODULE);
    let dependency = ident(Marker::Dependency.name());
    let plan_input = syn::Ident::new(Marker::PlanInput.name(), span);
    let source_module = syn::Ident::new(protocol::REFLECTION_MODULE, span);
    let compiler_key = compiler_key_tokens(key.as_ref());
    let type_end = crate::codegen::source::type_end(&service_type)
        .map(|end| crate::codegen::source::origin(OriginKind::InputTypeEnd, input_slot, "", end));
    let input_marker = quote_spanned!(span=>
        #source_module::#plan_input::<#service_type, #input_slot, #optional, #lazy>(#compiler_key, #label);
    );
    let key_origin = key
        .as_ref()
        .map(|key| crate::codegen::source::origin(OriginKind::InputKey, input_slot, "", key.span));
    zyn! {
        ::nestrs_core::activation::adapter::InputAdapter {
            service_type: {
                {{ reflection.clone() }}::{{ dependency }}::<{{ service_type.clone() }}, {{ input_slot }}>();
                {{ input_marker }}
                {{ type_end }}
                {{ key_origin }}
                ::nestrs_core::service::ServiceType::create::<{{ service_type.clone() }}>()
            },
            lazy: @RenderLazyInput(service_type = service_type.clone(), optional = optional, lazy = lazy),
            project: @RenderLazyProjection(service_type = service_type.clone(), lazy = lazy, is_trait_object = is_trait_object),
            prepare: @RenderDelivery(service_type = service_type.clone(), optional = optional, is_trait_object = is_trait_object),
        }
    }
}

/// 延迟 concrete 输入直接交付 token；显式 dyn 等待 binding 提供真实 coercion。
/// 类型别名先携带直接投影，计划选择 trait 路由时覆盖它，不能按源码拼写猜类型身份。
#[zyn::element]
fn render_lazy_projection(
    service_type: syn::Type,
    lazy: bool,
    is_trait_object: bool,
) -> zyn::TokenStream {
    zyn! {
        @if (*lazy && !*is_trait_object) {
            ::core::option::Option::Some(
                ::nestrs_core::activation::project_required::<{{ service_type }}>
                    as ::nestrs_core::activation::ServiceProjector
            )
        } @else {
            ::core::option::Option::None
        }
    }
}

/// 延迟包装和真实实例的投影是两个不同的阶段，不能用 lazy preparer 替换 delivery。
/// 前者创建可 await 的句柄；后者在目标完成后仍负责准确的 concrete/trait 类型恢复。
#[zyn::element]
fn render_lazy_input(service_type: syn::Type, optional: bool, lazy: bool) -> zyn::TokenStream {
    zyn! {
        @if (*lazy) {
            ::core::option::Option::Some(
                @if (*optional) {
                    ::nestrs_core::activation::prepare_lazy_optional::<{{ service_type }}>
                } @else {
                    ::nestrs_core::activation::prepare_lazy_required::<{{ service_type }}>
                }
                as ::nestrs_core::activation::LazyInputPreparer
            )
        } @else {
            ::core::option::Option::None
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
                ::core::option::Option::Some(
                    ::nestrs_core::activation::prepare_optional_absent::<{{ service_type }}>
                        as ::nestrs_core::activation::InputPreparer
                )
            } @else {
                ::core::option::Option::None
            }
        } @else {
            ::core::option::Option::Some(
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

/// 在真实类型化描述旁输出编译器所需的生命周期与选择策略。
#[zyn::element]
pub(crate) fn emit_plan_provider(
    service_type: zyn::TokenStream,
    key: Option<ServiceKeySpec>,
    lifetime: ServiceLifetime,
    primary: bool,
    lazy: Option<bool>,
) -> zyn::TokenStream {
    let lifetime_id = match lifetime {
        ServiceLifetime::Singleton => Lifetime::Singleton,
        ServiceLifetime::Scoped => Lifetime::Scoped,
        ServiceLifetime::Transient => Lifetime::Transient,
    } as u8;
    // 与工具内的初始化策略同源编码，driver 不从源码属性重新猜测策略。
    let initialization = Initialization::from_lazy(*lazy) as u8;
    let span = service_type.span();
    let reflection = syn::Ident::new(protocol::REFLECTION_MODULE, span);
    let provider = syn::Ident::new(Marker::PlanProvider.name(), span);
    let compiler_key = compiler_key_tokens(key.as_ref());
    let key_origin = key
        .as_ref()
        .map(|key| crate::codegen::source::origin(OriginKind::ProviderKey, 0, "", key.span));
    let type_end = crate::codegen::source::type_end(service_type)
        .map(|end| crate::codegen::source::origin(OriginKind::ProviderTypeEnd, 0, "", end));
    quote_spanned! {span=>
        #reflection::#provider::<#service_type, #lifetime_id, #primary, #initialization>(#compiler_key);
        #type_end
        #key_origin
    }
}

/// 将同一个静态 key 直接写入编译器 marker，供语义发现按精确身份选择蓝图。
#[zyn::element]
pub(crate) fn emit_compiler_key(key: Option<ServiceKeySpec>) -> zyn::TokenStream {
    compiler_key_tokens(key.as_ref())
}

fn compiler_key_tokens(key: Option<&ServiceKeySpec>) -> zyn::TokenStream {
    let span = key
        .map(|key| key.span)
        .unwrap_or_else(zyn::proc_macro2::Span::call_site);
    let reflection = syn::Ident::new(protocol::REFLECTION_MODULE, span);
    let compiler_key = syn::Ident::new(protocol::COMPILER_KEY, span);
    match key.map(|key| &key.kind) {
        Some(ServiceKeyKind::Named(name)) => {
            let name = syn::LitStr::new(name, span);
            quote_spanned!(span=> #reflection::#compiler_key::Named(#name))
        }
        Some(ServiceKeyKind::Indexed(index)) => {
            let index = syn::LitInt::new(&format!("{index}usize"), span);
            quote_spanned!(span=> #reflection::#compiler_key::Indexed(#index))
        }
        None => quote_spanned!(span=> #reflection::#compiler_key::Default),
    }
}

/// 生成 `ActivationAdapter::cleanup` 所需的零参数 async hook adapter。
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
            (|| -> ::nestrs_core::activation::adapter::CleanupFuture {
                ::std::boxed::Box::pin({{ cleanup_path }}())
            }) as ::nestrs_core::activation::adapter::CleanupHook
        )
    }
}
