//! `#[factory]` 的 Provider 代码生成。
//!
//! 本模块只消费 [`FactoryAnalysis`]，不会再查看用户函数参数上的属性。最终输出分成
//! 两个平级 element：一个重写后的用户函数，一个匿名 `const` 中的 adapter + linkme
//! callback。这样 factory 函数保留用户可以在模块内调用的普通函数语义，而 runtime
//! adapter 始终是不可从模块外命名的实现细节。

use crate::injection::{
    attrs::{cleanup::CleanupPath, lifetime::ServiceLifetime, service_key::ServiceKey},
    injectable::field_analyze::is_generic_concrete_type_path,
};

use super::{
    FactoryAnalysis, FactoryConfig, FactoryInvocation, FactoryParameterSpec, FactoryResultKind,
};
use zyn::{
    quote::quote,
    syn::{self, Type},
    zyn,
};

/// 渲染已移除参数 marker 且已经改写参数类型的用户 factory 函数。
///
/// 此 element 刻意不改变可见性；调用方应继续用既有 `MustBePrivateFn` element 包裹它，
/// 以维持 `#[factory]` 的模块私有约束。
#[zyn::element]
pub(crate) fn rewrite_factory_signature(analysis: FactoryAnalysis) -> zyn::TokenStream {
    let item = analysis.item.clone();

    zyn! {
        {{ item }}
    }
}

/// 生成隐藏 factory adapter 及写入统一 Provider linkme slice 的 callback。
///
/// adapter、cleanup wrapper 和 slice callback 被放入同一个私有且按 factory 名称唯一的
/// `const`。这样它在错误地出现在 impl 中时仍是合法的 associated const，同时
/// `__nestrs_factory_construct` 等真正的辅助函数继续停留在词法私有作用域，不会成为
/// 模块 API，也不会和其他 factory 的同名辅助符号冲突。
#[zyn::element]
pub(crate) fn emit_factory_provider(
    analysis: FactoryAnalysis,
    config: FactoryConfig,
    primary: bool,
) -> zyn::TokenStream {
    let factory = analysis.item.sig.ident.clone();
    let provider_const = zyn::format_ident!("__nestrs_factory_provider_for_{factory}");

    zyn! {
        #[doc(hidden)]
        const {{ provider_const }}: () = {
            @GenerateFactoryAdapter(
                analysis = analysis.clone(),
            )
            @EmitFactoryCleanupAdapter(
                cleanup = config.cleanup.clone(),
            )

            #[::nestrs_core::__private::linkme::distributed_slice(
                ::nestrs_core::__private::REFLECTED_PROVIDERS
            )]
            #[linkme(crate = ::nestrs_core::__private::linkme)]
            fn __nestrs_reflected_factory() -> ::nestrs_core::__private::Provider {
                ::nestrs_core::__private::Provider::Factory {
                    provide: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
                        @RenderServiceKey(key = config.key.clone()),
                        ::nestrs_core::registration::service_type::ServiceType::create::<{{ analysis.output.success_type.clone() }}>(),
                    ),
                    common: ::nestrs_core::__private::ProviderCommon {
                        lifetime: @RenderServiceLifetime(lifetime = config.lifetime),
                        primary: {{ primary }},
                        source: ::nestrs_core::registration::service_source::ServiceSource::new(
                            file!(),
                            line!(),
                            column!(),
                        ),
                        cleanup: @RenderFactoryCleanup(cleanup = config.cleanup.clone()),
                    },
                    dependencies: ::std::vec![
                        @for (parameter in analysis.parameters.iter()) {
                            @EmitFactoryParameterInjection(
                                parameter = parameter.clone(),
                            ),
                        }
                    ],
                    invoker: @RenderFactoryInvoker(
                        invocation = analysis.output.invocation,
                    ),
                }
            }

            ()
        };
    }
}

/// 为同步、`async fn` 与显式 Future factory 生成正确的构造 adapter。
#[zyn::element]
fn generate_factory_adapter(analysis: FactoryAnalysis) -> zyn::TokenStream {
    let invocation = analysis.output.invocation;
    let is_async = matches!(invocation, FactoryInvocation::Async);
    let context_binding = factory_context_binding(&analysis);

    zyn! {
        @if (is_async) {
            fn __nestrs_factory_construct(
                {{ context_binding.clone() }}
            ) -> ::nestrs_core::__private::ActivationFuture {
                ::std::boxed::Box::pin(async move {
                    @InvokeAsyncFactory(
                        function = analysis.item.sig.ident.clone(),
                        parameters = analysis.parameters.clone(),
                        result_kind = analysis.output.result_kind,
                    )
                })
            }
        } @else {
            fn __nestrs_factory_construct(
                {{ context_binding }}
            ) -> ::core::result::Result<
                ::nestrs_core::__private::ErasedService,
                ::nestrs_core::__private::ActivationError,
            > {
                @InvokeSyncFactory(
                    function = analysis.item.sig.ident.clone(),
                    parameters = analysis.parameters.clone(),
                    result_kind = analysis.output.result_kind,
                )
            }
        }
    }
}

/// 选择无输入时不会触发 unused-variable warning 的 context 参数写法。
fn factory_context_binding(analysis: &FactoryAnalysis) -> zyn::TokenStream {
    if analysis.parameters.is_empty() {
        quote!(_nestrs_factory_context: ::nestrs_core::__private::ConstructionContext)
    } else {
        quote!(mut __nestrs_factory_context: ::nestrs_core::__private::ConstructionContext)
    }
}

/// 生成同步 factory 调用与成功输出擦除。
#[zyn::element]
fn invoke_sync_factory(
    function: syn::Ident,
    parameters: Vec<FactoryParameterSpec>,
    result_kind: FactoryResultKind,
) -> zyn::TokenStream {
    let returns_result = matches!(result_kind, FactoryResultKind::Result);
    let context: syn::Ident = syn::parse_quote!(__nestrs_factory_context);
    let function_path = quote!(self::#function);

    zyn! {
        @if (returns_result) {
            match {{ function_path }}(
                @for (parameter in parameters.iter()) {
                    @TakeFactoryParameter(
                        parameter = parameter.clone(),
                        context = context.clone(),
                    ),
                }
            ) {
                ::core::result::Result::Ok(__nestrs_factory_service) => {
                    ::core::result::Result::Ok(
                        ::nestrs_core::__private::ErasedService::new(
                            __nestrs_factory_service
                        )
                    )
                }
                ::core::result::Result::Err(_) => {
                    ::core::result::Result::Err(
                        ::nestrs_core::__private::ActivationError::FactoryFailed {
                            provider: stringify!({{ function }}),
                            provider_source: ::nestrs_core::registration::service_source::ServiceSource::new(
                                file!(),
                                line!(),
                                column!(),
                            ),
                        }
                    )
                }
            }
        } @else {
            ::core::result::Result::Ok(
                ::nestrs_core::__private::ErasedService::new(
                    {{ function_path }}(
                        @for (parameter in parameters.iter()) {
                            @TakeFactoryParameter(
                                parameter = parameter.clone(),
                                context = context.clone(),
                            ),
                        }
                    )
                )
            )
        }
    }
}

/// 生成 `async fn` 或显式 Future factory 调用与成功输出擦除。
#[zyn::element]
fn invoke_async_factory(
    function: syn::Ident,
    parameters: Vec<FactoryParameterSpec>,
    result_kind: FactoryResultKind,
) -> zyn::TokenStream {
    let returns_result = matches!(result_kind, FactoryResultKind::Result);
    let context: syn::Ident = syn::parse_quote!(__nestrs_factory_context);
    let function_path = quote!(self::#function);

    zyn! {
        @if (returns_result) {
            match {{ function_path }}(
                @for (parameter in parameters.iter()) {
                    @TakeFactoryParameter(
                        parameter = parameter.clone(),
                        context = context.clone(),
                    ),
                }
            ).await {
                ::core::result::Result::Ok(__nestrs_factory_service) => {
                    ::core::result::Result::Ok(
                        ::nestrs_core::__private::ErasedService::new(
                            __nestrs_factory_service
                        )
                    )
                }
                ::core::result::Result::Err(_) => {
                    ::core::result::Result::Err(
                        ::nestrs_core::__private::ActivationError::FactoryFailed {
                            provider: stringify!({{ function }}),
                            provider_source: ::nestrs_core::registration::service_source::ServiceSource::new(
                                file!(),
                                line!(),
                                column!(),
                            ),
                        }
                    )
                }
            }
        } @else {
            ::core::result::Result::Ok(
                ::nestrs_core::__private::ErasedService::new(
                    {{ function_path }}(
                        @for (parameter in parameters.iter()) {
                            @TakeFactoryParameter(
                                parameter = parameter.clone(),
                                context = context.clone(),
                            ),
                        }
                    ).await
                )
            )
        }
    }
}

/// 从预绑定 `ConstructionContext` 取出一个 factory 依赖参数。
///
/// 这里直接把 `take` 表达式作为 factory 调用实参，避免生成固定局部变量名与用户的
/// 简单参数标识符发生碰撞。
#[zyn::element]
fn take_factory_parameter(
    parameter: FactoryParameterSpec,
    context: syn::Ident,
) -> zyn::TokenStream {
    let service_type = parameter.service_type.clone();
    let position = parameter.input_position;
    let optional = parameter.optional;

    zyn! {
        @if (optional) {
            {{ context }}.take_optional::<{{ service_type }}>(
                ::nestrs_core::__private::InputPosition({{ position }})
            )?
        } @else {
            {{ context }}.take::<{{ service_type }}>(
                ::nestrs_core::__private::InputPosition({{ position }})
            )?
        }
    }
}

/// 为可选 cleanup path 生成类型擦除的 `CleanupHook` adapter。
#[zyn::element]
fn emit_factory_cleanup_adapter(cleanup: Option<CleanupPath>) -> zyn::TokenStream {
    let cleanup_path = cleanup.as_ref().map(|cleanup| cleanup.func_path.clone());

    zyn! {
        @if (cleanup_path.is_some()) {
            fn __nestrs_factory_cleanup() -> ::nestrs_core::__private::CleanupFuture {
                ::std::boxed::Box::pin(async move {
                    {{ cleanup_path.clone().unwrap() }}().await;
                })
            }
        }
    }
}

/// 渲染一个 factory 参数的统一 `InjectionSpec`。
#[zyn::element]
fn emit_factory_parameter_injection(parameter: FactoryParameterSpec) -> zyn::TokenStream {
    let service_type = parameter.service_type.clone();
    let key = parameter.key.clone();
    let declaration_position = parameter.declaration_position;
    let input_position = parameter.input_position;
    let ident = parameter.ident.clone();
    let optional = parameter.optional;
    let is_concrete = matches!(service_type, Type::Path(_));
    let is_trait_object = matches!(service_type, Type::TraitObject(_));
    let has_closed_provider = is_generic_concrete_type_path(&service_type);

    zyn! {
        ::nestrs_core::__private::InjectionSpec {
            declaration_position: {{ declaration_position }},
            input_position: ::nestrs_core::__private::InputPosition({{ input_position }}),
            token: ::nestrs_core::registration::service_identifier::ServiceIdentifier::new(
                @RenderServiceKey(key = key.clone()),
                ::nestrs_core::registration::service_type::ServiceType::create::<{{ service_type.clone() }}>(),
            ),
            optional: {{ optional }},
            label: ::core::option::Option::Some(stringify!({{ ident }})),
            target: @RenderInjectionTarget(
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            prepare_input: @RenderFactoryParameterPreparer(
                service_type = service_type.clone(),
                optional = optional,
                is_concrete = is_concrete,
                is_trait_object = is_trait_object,
            ),
            closed_provider: @RenderClosedProviderCallback(
                service_type = service_type,
                has_closed_provider = has_closed_provider,
            ),
        }
    }
}

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

/// concrete 参数直接使用 monomorphized preparer；trait-object 参数必须等待 bind
/// provider 的 typed projector。optional trait 在没有 bind 时可以安全地准备 `None`。
#[zyn::element]
fn render_factory_parameter_preparer(
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

#[zyn::element]
fn render_factory_invoker(invocation: FactoryInvocation) -> zyn::TokenStream {
    let is_async = matches!(invocation, FactoryInvocation::Async);

    zyn! {
        @if (is_async) {
            ::nestrs_core::__private::FactoryInvoker::Async(__nestrs_factory_construct)
        } @else {
            ::nestrs_core::__private::FactoryInvoker::Sync(__nestrs_factory_construct)
        }
    }
}

#[zyn::element]
fn render_factory_cleanup(cleanup: Option<CleanupPath>) -> zyn::TokenStream {
    zyn! {
        @if (cleanup.is_some()) {
            ::core::option::Option::Some(
                __nestrs_factory_cleanup as ::nestrs_core::__private::CleanupHook
            )
        } @else {
            ::core::option::Option::None
        }
    }
}

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

#[zyn::element]
fn render_service_lifetime(lifetime: ServiceLifetime) -> zyn::TokenStream {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::factory::analyze::analyze_factory;
    use zyn::{Render, syn};

    fn render(source: &str) -> String {
        let analysis = analyze_factory(syn::parse_str(source).expect("factory should parse"))
            .expect("factory should analyze");
        EmitFactoryProvider {
            analysis,
            config: FactoryConfig {
                lifetime: ServiceLifetime::Scoped,
                key: Some(ServiceKey::Named("writer".to_owned())),
                cleanup: None,
            },
            primary: true,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn emits_a_hidden_sync_provider_with_parameter_injections() {
        let output = render(
            r#"
            fn make(
                database: Database,
                #[inject(key = "audit")]
                audit: Option<dyn Audit>,
            ) -> Result<Service, Error> { todo!() }
            "#,
        );

        assert!(output.contains("const __nestrs_factory_provider_for_make : ()"));
        assert!(output.contains("REFLECTED_PROVIDERS"));
        assert!(output.contains("Provider :: Factory"));
        assert!(output.contains("FactoryInvoker :: Sync"));
        assert!(output.contains("take :: < Database >"));
        assert!(output.contains("take_optional :: < dyn Audit >"));
        assert!(output.contains("ServiceKey :: Named (\"audit\")"));
        assert!(output.contains("declaration_position : 1usize"));
        assert!(output.contains("primary : true"));
        assert!(output.contains("FactoryFailed"));
    }

    #[test]
    fn emits_an_async_invoker_for_async_and_explicit_future_factories() {
        let async_output = render("async fn make() -> Service { todo!() }");
        assert!(async_output.contains("FactoryInvoker :: Async"));
        assert!(async_output.contains("Box :: pin (async move"));
        assert!(async_output.contains("make"));
        assert!(async_output.contains(". await"));

        let future_output = render(
            "fn make() -> impl ::core::future::Future<Output = Result<Service, Error>> { todo!() }",
        );
        assert!(future_output.contains("FactoryInvoker :: Async"));
        assert!(future_output.contains("make"));
        assert!(future_output.contains(". await"));
    }
}
