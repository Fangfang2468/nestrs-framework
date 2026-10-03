//! `#[factory]` 的 Provider 代码生成。
//!
//! 本模块只消费 [`FactoryAnalysis`]，不会再查看用户函数参数上的属性。最终输出分成
//! 两个平级 element：一个重写后的用户函数，一个匿名 `const` 中的 adapter + 描述回调
//! callback。这样 factory 函数保留用户可以在模块内调用的普通函数语义，而 runtime
//! adapter 始终是不可从模块外命名的实现细节。

use crate::codegen::injection::render::{
    EmitCompilerKey, EmitDependencyRequest, EmitPlanProvider, RenderCleanupHook,
};

use super::{
    FactoryAnalysis, FactoryConfig, FactoryInvocation, FactoryParameterSpec, FactoryResultKind,
};
use crate::{
    codegen::reflection::ident,
    protocol::{self, Marker},
};
use zyn::{quote::quote, syn, zyn};

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

/// 生成隐藏 factory adapter 及写入统一 Provider 描述清单 的 callback。
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
    lazy: Option<bool>,
) -> zyn::TokenStream {
    let factory = analysis.item.sig.ident.clone();
    let success_type = &analysis.output.success_type;
    let service_type = quote!(#success_type);
    let async_factory = matches!(analysis.output.invocation, FactoryInvocation::Async);
    let reflection = crate::codegen::reflection::support(false);
    let reflection_module = ident(protocol::REFLECTION_MODULE);
    let factory_marker = ident(Marker::PlanFactory.name());
    let provider_marker = ident(Marker::Provider.name());
    let provider_const = zyn::format_ident!("__nestrs_factory_provider_for_{factory}");

    zyn! {
        #[doc(hidden)]
        #[allow(clippy::unused_unit)]
        const {{ provider_const }}: () = {
            {{ reflection }}
            @GenerateFactoryAdapter(
                analysis = analysis.clone(),
            )
        #[allow(dead_code)]
            fn __nestrs_reflected_factory() -> ::nestrs_core::activation::adapter::ActivationAdapter {
                {{ reflection_module.clone() }}::{{ factory_marker }}::<{{ async_factory }}>();
                {{ reflection_module }}::{{ provider_marker }}::<{{ analysis.output.success_type.clone() }}>(
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
                        service_type: ::nestrs_core::service::ServiceType::create::<{{ analysis.output.success_type.clone() }}>(),
                        cleanup: @RenderCleanupHook(cleanup = config.cleanup.clone()),
                        inputs: ::std::vec![
                            @for (parameter in analysis.parameters.iter()) {
                                @EmitDependencyRequest(request = parameter.dependency_request()),
                            }
                        ],
                        constructor: ::nestrs_core::activation::adapter::Constructor::Factory(@RenderFactoryInvoker(
                            invocation = analysis.output.invocation,
                        )),
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
    let context_binding = factory_context_binding(analysis);

    zyn! {
        @if (is_async) {
            fn __nestrs_factory_construct<'frame>(
                {{ context_binding.clone() }}
            ) -> ::nestrs_core::activation::FactoryFuture<'frame> {
                ::std::boxed::Box::pin(async move {
                    @InvokeAsyncFactory(
                        function = analysis.item.sig.ident.clone(),
                        parameters = analysis.parameters.clone(),
                        result_kind = analysis.output.result_kind,
                    )
                })
            }
        } @else {
            fn __nestrs_factory_construct<'frame>(
                {{ context_binding }}
            ) -> ::core::result::Result<
                ::nestrs_core::activation::ErasedService,
                ::nestrs_core::activation::ConstructionError,
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
        quote!(
            __nestrs_factory_context:
                ::nestrs_core::activation::FactoryInputs<'frame>
        )
    } else {
        quote!(
            mut __nestrs_factory_context:
                ::nestrs_core::activation::FactoryInputs<'frame>
        )
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
    let parameter_bindings: Vec<_> = parameters
        .iter()
        .map(|parameter| factory_input_binding_identifier(parameter.input_slot))
        .collect();

    zyn! {
        @for (parameter in parameters.iter()) {
            @BindFactoryParameter(
                parameter = parameter.clone(),
                context = context.clone(),
            )
        }
        {{ context }}.ensure_all_consumed()?;
        @if (returns_result) {
            match {{ function_path }}(
                @for (binding in parameter_bindings.iter()) {
                    {{ binding }},
                }
            ) {
                ::core::result::Result::Ok(__nestrs_factory_service) => {
                    ::core::result::Result::Ok(
                        ::nestrs_core::activation::ErasedService::new(
                            __nestrs_factory_service
                        )
                    )
                }
                ::core::result::Result::Err(__nestrs_factory_error) => {
                    ::core::result::Result::Err(
                        ::nestrs_core::activation::ConstructionError::FactoryFailed {
                            provider: stringify!({{ function }}),
                            detail: ::std::format!("{:?}", __nestrs_factory_error),
                            provider_source: ::nestrs_core::service::ServiceSource::new(
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
                ::nestrs_core::activation::ErasedService::new(
                    {{ function_path }}(
                        @for (binding in parameter_bindings.iter()) {
                            {{ binding }},
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
    let parameter_bindings: Vec<_> = parameters
        .iter()
        .map(|parameter| factory_input_binding_identifier(parameter.input_slot))
        .collect();

    zyn! {
        @for (parameter in parameters.iter()) {
            @BindFactoryParameter(
                parameter = parameter.clone(),
                context = context.clone(),
            )
        }
        {{ context }}.ensure_all_consumed()?;
        @if (returns_result) {
            match {{ function_path }}(
                @for (binding in parameter_bindings.iter()) {
                    {{ binding }},
                }
            ).await {
                ::core::result::Result::Ok(__nestrs_factory_service) => {
                    ::core::result::Result::Ok(
                        ::nestrs_core::activation::ErasedService::new(
                            __nestrs_factory_service
                        )
                    )
                }
                ::core::result::Result::Err(__nestrs_factory_error) => {
                    ::core::result::Result::Err(
                        ::nestrs_core::activation::ConstructionError::FactoryFailed {
                            provider: stringify!({{ function }}),
                            detail: ::std::format!("{:?}", __nestrs_factory_error),
                            provider_source: ::nestrs_core::service::ServiceSource::new(
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
                ::nestrs_core::activation::ErasedService::new(
                    {{ function_path }}(
                        @for (binding in parameter_bindings.iter()) {
                            {{ binding }},
                        }
                    ).await
                )
            )
        }
    }
}

/// 从 frame-bound `FactoryInputs` 取出一个 factory 依赖参数。
///
/// 适配器先把所有输入写入只属于 adapter 的按槽位命名局部变量，确认没有 metadata
/// 遗留槽位后才调用用户 factory。返回的 `&'frame T` 会沿着 adapter future 保持到
/// factory 完成，从而不能逃逸至输出服务或后台任务。延迟输入则从槽位按值移出；其
/// 生命周期由句柄的 owner 访问协议和成功实例 lease 管理，可以安全保存到返回服务。
#[zyn::element]
fn take_factory_parameter(
    parameter: FactoryParameterSpec,
    context: syn::Ident,
) -> zyn::TokenStream {
    let service_type = parameter.service_type.clone();
    let slot = parameter.input_slot;
    let optional = parameter.optional;
    let lazy = parameter.lazy;

    zyn! {
        @if (lazy) {
            @if (optional) {
                {{ context }}.take_optional_lazy::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ slot }})
                )?
            } @else {
                {{ context }}.take_lazy::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ slot }})
                )?
            }
        } @else {
            @if (optional) {
                {{ context }}.take_optional::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ slot }})
                )?
            } @else {
                {{ context }}.take::<{{ service_type }}>(
                    ::nestrs_core::activation::InputSlot::new({{ slot }})
                )?
            }
        }
    }
}

#[zyn::element]
fn bind_factory_parameter(
    parameter: FactoryParameterSpec,
    context: syn::Ident,
) -> zyn::TokenStream {
    let binding = factory_input_binding_identifier(parameter.input_slot);

    zyn! {
        let {{ binding }} = @TakeFactoryParameter(
            parameter = parameter.clone(),
            context = context.clone(),
        );
    }
}

fn factory_input_binding_identifier(input_slot: usize) -> syn::Ident {
    zyn::format_ident!("__nestrs_factory_input_{input_slot}")
}

#[zyn::element]
fn render_factory_invoker(invocation: FactoryInvocation) -> zyn::TokenStream {
    let is_async = matches!(invocation, FactoryInvocation::Async);

    zyn! {
        @if (is_async) {
            ::nestrs_core::activation::adapter::FactoryInvoker::Async(__nestrs_factory_construct)
        } @else {
            ::nestrs_core::activation::adapter::FactoryInvoker::Sync(__nestrs_factory_construct)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::injection::{
        macros::factory::analyze::analyze_factory,
        macros_attrs::{lifetime::ServiceLifetime, service_key::ServiceKeySpec},
    };
    use zyn::{Render, syn};

    fn render(source: &str) -> String {
        let analysis = analyze_factory(syn::parse_str(source).expect("factory should parse"))
            .expect("factory should analyze");
        EmitFactoryProvider {
            analysis,
            config: FactoryConfig {
                lifetime: ServiceLifetime::Scoped,
                key: Some(ServiceKeySpec::Named("writer".to_owned())),
                cleanup: None,
            },
            primary: true,
            lazy: None,
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
                #[inject("audit")]
                audit: Option<dyn Audit>,
            ) -> Result<Service, Error> { todo!() }
            "#,
        );

        assert!(output.contains("const __nestrs_factory_provider_for_make : ()"));
        assert!(output.contains("compiler_provider"));
        assert!(output.contains("Constructor :: Factory"));
        assert!(output.contains("FactoryInvoker :: Sync"));
        assert!(output.contains("FactoryInputs < 'frame >"));
        assert!(output.contains("take :: < Database >"));
        assert!(output.contains("take_optional :: < dyn Audit >"));
        assert!(output.contains("let __nestrs_factory_input_0 ="));
        assert!(output.contains("let __nestrs_factory_input_1 ="));
        assert!(output.contains("ensure_all_consumed ()"));
        let first_input = output
            .find("let __nestrs_factory_input_0")
            .expect("the first parameter should be materialized into a private local");
        let second_input = output
            .find("let __nestrs_factory_input_1")
            .expect("the second parameter should be materialized into a private local");
        let ensure = output
            .find("ensure_all_consumed ()")
            .expect("the factory adapter should validate input coverage");
        let invoke = output
            .find("self :: make")
            .expect("the user factory should only be invoked after input validation");
        assert!(first_input < second_input && second_input < ensure && ensure < invoke);
        assert!(output.contains("CompilerKey :: Named"));
        assert!(!output.contains("ServiceIdentifier"));
        assert!(output.contains("\"audit\""));
        assert!(output.contains("compiler_plan_input :: < dyn Audit , 1usize , true , false >"));
        assert!(output.contains("compiler_plan_provider :: < Service , 1u8 , true , 0u8 >"));
        assert!(output.contains("FactoryFailed"));
    }

    #[test]
    fn emits_an_async_invoker_for_async_and_explicit_future_factories() {
        let async_output = render("async fn make() -> Service { todo!() }");
        assert!(async_output.contains("FactoryInvoker :: Async"));
        assert!(async_output.contains("FactoryFuture < 'frame >"));
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

    #[test]
    fn lazy_parameter_ownership_is_preserved_for_every_factory_invocation_shape() {
        for source in [
            "fn make(#[lazy] dependency: Dependency) -> Service { todo!() }",
            "async fn make(#[lazy] dependency: Dependency) -> Result<Service, Error> { todo!() }",
            "fn make(#[lazy] dependency: Dependency) -> impl ::core::future::Future<Output = Service> { todo!() }",
        ] {
            let output = render(source);
            assert!(output.contains("take_lazy :: < Dependency >"), "{output}");
            assert!(!output.contains("take :: < Dependency >"), "{output}");
            assert!(
                output.contains("prepare_lazy_required :: < Dependency >"),
                "{output}"
            );
            let bind = output.find("let __nestrs_factory_input_0").unwrap();
            let verify = output.find("ensure_all_consumed ()").unwrap();
            let invoke = output.find("self :: make").unwrap();
            assert!(bind < verify && verify < invoke, "{output}");
        }
    }
}
