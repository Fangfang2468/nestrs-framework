//! Nestrs 声明展开的共享编译期实现。
//!
//! 为属性过程宏与编译器工具链提供同一套分析、重写和代码生成逻辑。
//! 此 crate 不执行服务构造，也不依赖 DI runtime；生成的 Rust 代码仍引用
//! core 实际所属私有模块；编译器限制这些引用只来自真实生成代码。
//!
//! 输入直接使用带 span 的 `proc_macro2::TokenStream`，可以在普通进程中调用，
//! 无需过程宏执行上下文，也不通过字符串往返解析 token。

mod conditional_fields;
mod constructor;
mod constructor_codegen;
mod constructor_ide;
mod injection;
mod reflection;
mod source;

mod utility;

#[cfg(test)]
mod lazy_tests;
#[cfg(test)]
mod tests;

use constructor::ConstructorMode;
use zyn::{
    meta::Args,
    syn::{self, spanned::Spanned},
    zyn,
};

use crate::codegen::injection::{
    macros::{
        bind::EmitBoundProvider,
        factory::{
            EmitFactoryProvider, RewriteFactorySignature, analyze_factory, parse_factory_config,
        },
        injectable::{
            CollectInjectableProvider, DefineGenericInjectableProvider, EmitInjectableRegistration,
            GenerateInjectableConstructor, analyze_fields, config::InjectableConfig,
            rewrite_injection_field,
        },
    },
    macros_attrs::lazy::{ServiceLazyConfig, defer_to_provider, take_lazy_for_provider},
    macros_attrs::primary::{
        DeferPrimaryToFactory, DeferPrimaryToInjectable, PrimaryConfig, has_attribute_named,
        take_primary_for_factory, take_primary_for_injectable,
    },
};
use crate::codegen::utility::{
    CheckInterfaceType, MustBePrivateFn, RejectUnsafeAndExternFn, RejectUnsafeImpl,
    RequireModuleScope, RequireNonUnitFutureOutputType, RequireNonUnitResultOkType,
    RequireNonUnitReturnType, impl_self_ident,
};

/// 将模块作用域内的结构体标记为可注入服务。
///
/// 字段来源由属性决定：`#[inject]` 从容器输入取得只读 `Injection<T>` 令牌；
/// 配合字段级 `#[lazy]` 时改为 `LazyInjection<T>`，通过 `get().await` 延迟获取目标；
/// `#[value(<Rust expression>)]` 则在词法隔离的隐藏构造 adapter 被 container
/// 调用时求值。因此它统一支持字面量、模块常量/静态项、可见路径、函数调用与普通
/// 组合表达式，并由 Rust 完成名称解析和类型检查。字符串字面量及模块常量/静态项
/// 会在需要时通过 `Into<字段类型>` 转换，因此 `String` 字段可直接写
/// `#[value("name")]`。
///
/// 该 adapter 不是类型成员且只由编译器收集的 Provider 持有函数指针，因而用户不能以
/// `Service::__nestrs_construct(...)` 调用。由于它仍是非捕获函数，`#[value]` 不能
/// 引用调用点局部变量或另一字段；表达式必须能转换为字段类型。
fn injectable(
    item: syn::ItemStruct,
    args: Args,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::Output {
    let input = syn::Item::Struct(item.clone());
    // 1) 严格校验参数形态：只允许命名参数，且不允许重复
    let mut seen: Vec<String> = Vec::new();

    for arg in args.iter() {
        let Some(name) = arg.name() else {
            return zyn::mark::error("`#[injectable]` 参数填写格式错误")
                .span(arg.span())
                .build()
                .emit()
                .into();
        };

        let name = name.to_string();
        if seen.contains(&name) {
            return zyn::mark::error(format!("`#[injectable]` 参数 `{name}` 重复声明"))
                .span(arg.span())
                .build()
                .emit()
                .into();
        }
        seen.push(name);
    }

    // 2) 强类型解析参数；失败时直接发射诊断
    let config = match InjectableConfig::from_args(&args) {
        Ok(cfg) => cfg,
        Err(diag) => return diag.emit().into(),
    };

    // 属性宏按源码顺序展开。若下方还有 `#[primary]`，它尚未执行，必须由
    // `injectable` 直接解析并移除；若上方的 `primary` 已执行，则这里消费它
    // 留下的私有 marker。两者都在字段分析前完成，避免 marker 泄漏到最终 AST。
    let mut item = item;
    let primary = match take_primary_for_injectable(&mut item.attrs) {
        Ok(primary) => primary,
        Err(error) => return error.into_compile_error().into(),
    };
    let source =
        source::ProviderOrigin::from_args(item.ident.clone(), &args, primary.source_span());
    let primary_attribute_use = primary.consumed_attribute_use();
    let lazy = match take_lazy_for_provider(&mut item.attrs) {
        Ok(lazy) => lazy,
        Err(error) => return error.into_compile_error().into(),
    };
    let lazy_attribute_use = lazy.consumed_attribute_use();

    // 分析阶段只产出共享数据和去除 marker 的 AST；它不负责渲染后续阶段。
    let analyzed_fields = match analyze_fields(item) {
        Ok(fields) => fields,
        Err(error) => return error.into_compile_error().into(),
    };
    let ide_selection = match constructor_ide::selection(&analyzed_fields.item) {
        Ok(selection) => selection,
        Err(error) => return error.into_compile_error().into(),
    };
    let constructor_mode = match &ide_selection {
        None => ConstructorMode::Deferred,
        Some(selection) if selection.constructor => ConstructorMode::Explicit,
        Some(_) => ConstructorMode::Automatic,
    };
    let declaration = match rewrite_injection_field(&analyzed_fields, ide_selection.as_ref()) {
        Ok(declaration) => declaration,
        Err(error) => return error.into_compile_error().into(),
    };

    // 模块作用域检查所需的标识符必须在 zyn element 消费 AST 前保存。
    let scope_ident = Some(analyzed_fields.item.ident.clone());
    let is_open_generic_provider = !analyzed_fields.item.generics.params.is_empty();

    // 字段定义、构造 adapter 与 Provider 注册是三个独立的输出职责。注册 scope 只
    // 接收后两者作为 children，明确它们必须共享匿名词法作用域，避免把 helper
    // 暴露为用户可调用的 inherent method。
    zyn! {
        @RequireModuleScope(ident = scope_ident) {
            {{ declaration }}
            @if (is_open_generic_provider) {
                {{ primary_attribute_use }}
                {{ lazy_attribute_use }}
                @DefineGenericInjectableProvider(
                    binding_span = binding_span,
                    analysis = analyzed_fields,
                    config = config,
                    primary = primary.is_primary(),
                    source = source.clone(),
                    lazy = lazy.value(),
                    mode = constructor_mode,
                )
            } @else {
                @EmitInjectableRegistration {
                    {{ primary_attribute_use }}
                    {{ lazy_attribute_use }}
                    @GenerateInjectableConstructor(
                        binding_span = binding_span,
                        analysis = analyzed_fields.clone(),
                        mode = constructor_mode,
                    )
                    @CollectInjectableProvider(
                        analysis = analyzed_fields,
                        config = config,
                        primary = primary.is_primary(),
                        source = source.clone(),
                        lazy = lazy.value(),
                        mode = constructor_mode,
                    )
                }
            }
        }
    }
}

/// 将模块作用域内的私有函数声明为服务工厂函数。
///
/// 支持同步与 `async` 函数；不允许带 `self`、`unsafe` 或 `extern` 的函数。直接返回值、
/// `Result` 的 `Ok` 类型与显式 `Future::Output` 均不能为 `()`。
fn factory(item: syn::ItemFn, args: Args, binding_span: zyn::proc_macro2::Span) -> zyn::Output {
    let input = syn::Item::Fn(item.clone());
    let config = match parse_factory_config(&args) {
        Ok(config) => config,
        Err(error) => return error.emit().into(),
    };

    // `factory` 与 `injectable` 使用同一个 primary 交接模式：若 primary 位于下方，
    // 这里直接消费原属性；若 primary 位于上方，则消费 primary 宏留下的私有 marker。
    // 这一步必须发生在签名分析前，避免 marker 被当成非法参数属性或泄漏到最终函数。
    let mut item = item;
    let primary = match take_primary_for_factory(&mut item.attrs) {
        Ok(primary) => primary,
        Err(error) => return error.into_compile_error().into(),
    };
    let source =
        source::ProviderOrigin::from_args(item.sig.ident.clone(), &args, primary.source_span());
    let primary_attribute_use = primary.consumed_attribute_use();
    let lazy = match take_lazy_for_provider(&mut item.attrs) {
        Ok(lazy) => lazy,
        Err(error) => return error.into_compile_error().into(),
    };
    let lazy_attribute_use = lazy.consumed_attribute_use();

    let analysis = match analyze_factory(item) {
        Ok(analysis) => analysis,
        Err(error) => return error.into_compile_error().into(),
    };
    let scope_ident = Some(analysis.item.sig.ident.clone());

    zyn! {
        @RejectUnsafeAndExternFn(macro_name = "factory".to_string(), item = analysis.item.clone()) {
            @RequireNonUnitReturnType(macro_name = "factory".to_string(), item = analysis.item.clone()) {
                @RequireNonUnitResultOkType(macro_name = "factory".to_string(), item = analysis.item.clone()) {
                    @RequireNonUnitFutureOutputType(macro_name = "factory".to_string(), item = analysis.item.clone()) {
                        @RequireModuleScope(ident = scope_ident) {
                            {{ primary_attribute_use }}
                            {{ lazy_attribute_use }}
                            @MustBePrivateFn() {
                                @RewriteFactorySignature(
                                    analysis = analysis.clone(),
                                )
                            }
                            @EmitFactoryProvider(
                                analysis = analysis.clone(),
                                binding_span = binding_span,
                                config = config,
                                primary = primary.is_primary(),
                                source = source.clone(),
                                lazy = lazy.value(),
                            )
                        }
                    }
                }
            }
        }
    }
}

/// 将函数或结构体标记为主实现，不接受任何参数。
///
/// 标注普通函数时，条件与约束和 `#[factory]` 相同（模块作用域、私有、非
/// `unsafe`/`extern`，支持同步与 `async`）；标注结构体时，条件与约束和
/// `#[injectable]` 相同（仅限模块作用域）。
fn primary(item: syn::Item, args: Args) -> zyn::Output {
    let input = item.clone();
    let macro_name = "primary".to_owned();
    let primary_config = PrimaryConfig::from_args(&args);
    let function_has_factory = matches!(
        &item,
        syn::Item::Fn(function) if has_attribute_named(&function.attrs, "factory")
    );

    fn reject(span: ::zyn::proc_macro2::Span, message: &str) -> ::zyn::proc_macro2::TokenStream {
        syn::Error::new(span, message).into_compile_error()
    }

    zyn! {
        @match (primary_config) {
            Ok(primary) => {
                @match (item) {
                    // 函数标记：条件与 `#[factory]` 相同。
                    syn::Item::Fn(item) => {
                        // `primary` 在 `factory` 上方时，factory 尚未展开。只追加
                        // 内部 marker，交由 factory 统一生成一个 Provider::Factory；
                        // 不能在此处独立生成注册项。
                        @if (function_has_factory) {
                            @DeferPrimaryToFactory(
                                item = item.clone(),
                                primary = primary.clone(),
                            )
                        } @else {
                            @if (item.sig.receiver().is_some()) {
                                {{ reject(item.sig.ident.span(), "`#[primary]` 只能标注普通函数，不能用于带 `self` 的 impl 方法") }}
                            } @else {
                                @RejectUnsafeAndExternFn(macro_name = macro_name.clone(), item = item.clone()) {
                                    @RequireModuleScope(ident = Some(item.sig.ident.clone())) {
                                        @MustBePrivateFn() {
                                            {{ item }}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // `primary` 位于 `injectable` 上方时，后者尚未展开。此
                    // element 追加 marker，交由 injectable 收集为 ProviderCommon::primary。
                    syn::Item::Struct(item) => {
                        @RequireModuleScope(ident = Some(item.ident.clone())) {
                            @DeferPrimaryToInjectable(
                                item = item.clone(),
                                primary = primary.clone(),
                            )
                        }
                    }
                    other => {
                        {{ reject(other.span(), "`#[primary]` 只能标注普通函数或结构体") }}
                    }
                }
            }
            Err(error) => {
                {{ error.into_compile_error() }}
            }
        }
    }
}

fn bind(item: syn::ItemImpl, args: Args) -> zyn::Output {
    let input = syn::Item::Impl(item.clone());
    if let Some(arg) = args.iter().next() {
        return syn::Error::new(arg.span(), "`#[bind]` 不接受参数")
            .into_compile_error()
            .into();
    }

    let interface = match &item.trait_ {
        Some((None, interface, _)) => interface.clone(),
        Some((Some(_), _, _)) => {
            return syn::Error::new(item.span(), "`#[bind]` 不支持负 trait impl")
                .into_compile_error()
                .into();
        }
        None => {
            return syn::Error::new(
                item.span(),
                "`#[bind]` 只能标注 trait impl（例如 `impl Trait for Service`）",
            )
            .into_compile_error()
            .into();
        }
    };

    if !item.generics.params.is_empty() {
        return syn::Error::new(
            item.generics.span(),
            "`#[bind]` 不支持泛型 impl；请绑定具体的服务类型",
        )
        .into_compile_error()
        .into();
    }

    let service = item.self_ty.clone();

    zyn! {
        @RejectUnsafeImpl(macro_name = "bind".to_string(), item = item.clone()) {
            @RequireModuleScope(ident = impl_self_ident(&item)) {
                @CheckInterfaceType(interface = interface.clone()) {
                    {{ item }}
                    @EmitBoundProvider(
                        service = (*service).clone(),
                        interface = interface.clone(),
                    )
                }
            }
        }
    }
}

/// 展开 `#[injectable]`，供过程宏和编译器适配层共享。
#[doc(hidden)]
pub fn expand_injectable(args: zyn::TokenStream, input: zyn::TokenStream) -> zyn::TokenStream {
    expand_injectable_with_binding_span(args, input, zyn::proc_macro2::Span::mixed_site())
}

/// 私有 bridge 显式提供定义处卫生；仅生成绑定使用它，业务 token 保留原 span。
/// 普通进程中的 token 单测仍可使用上面的便捷入口，不需要 proc_macro 运行环境。
#[doc(hidden)]
pub fn expand_injectable_with_binding_span(
    args: zyn::TokenStream,
    input: zyn::TokenStream,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    if let Ok(item) = syn::parse2::<syn::ItemStruct>(input.clone())
        && conditional_fields::needs_filtering(&item)
    {
        return conditional_fields::defer(args, item, binding_span)
            .unwrap_or_else(syn::Error::into_compile_error);
    }
    expand_attribute(args, input, |item, args| {
        injectable(item, args, binding_span)
    })
}

/// 由标准 derive 展开取得 rustc 已筛选的字段，再调用同一声明后端。
#[doc(hidden)]
pub fn expand_configured_injectable(input: zyn::TokenStream) -> zyn::TokenStream {
    expand_configured_injectable_with_binding_span(input, zyn::proc_macro2::Span::mixed_site())
}

/// cfg 筛选后的 derive 入口使用自身的定义处卫生，与直接声明走同一生成后端。
#[doc(hidden)]
pub fn expand_configured_injectable_with_binding_span(
    input: zyn::TokenStream,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    match conditional_fields::restore(input) {
        Ok((args, item)) => expand_attribute(args, zyn::quote::quote!(#item), |item, args| {
            injectable(item, args, binding_span)
        }),
        Err(error) => error.into_compile_error(),
    }
}

/// 展开 `#[factory]`，供过程宏和编译器适配层共享。
#[doc(hidden)]
pub fn expand_factory(args: zyn::TokenStream, input: zyn::TokenStream) -> zyn::TokenStream {
    expand_factory_with_binding_span(args, input, zyn::proc_macro2::Span::mixed_site())
}

/// factory 内部 provider 项使用定义处卫生，避免占用业务模块的值命名空间。
#[doc(hidden)]
pub fn expand_factory_with_binding_span(
    args: zyn::TokenStream,
    input: zyn::TokenStream,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    expand_attribute(args, input, |item, args| factory(item, args, binding_span))
}

/// 展开同步关联构造函数；所属服务身份由编译器在名称解析后关联。
#[doc(hidden)]
pub fn expand_constructor(args: zyn::TokenStream, input: zyn::TokenStream) -> zyn::TokenStream {
    expand_constructor_with_binding_span(args, input, zyn::proc_macro2::Span::mixed_site())
}

/// 显式 constructor 只对生成输入、结果和错误绑定使用 bridge 提供的卫生上下文。
#[doc(hidden)]
pub fn expand_constructor_with_binding_span(
    args: zyn::TokenStream,
    input: zyn::TokenStream,
    binding_span: zyn::proc_macro2::Span,
) -> zyn::TokenStream {
    constructor_codegen::expand(args, input, binding_span)
        .unwrap_or_else(syn::Error::into_compile_error)
}

/// 展开 `#[primary]`，包括其与服务声明之间的属性交接。
#[doc(hidden)]
pub fn expand_primary(args: zyn::TokenStream, input: zyn::TokenStream) -> zyn::TokenStream {
    expand_attribute(args, input, primary)
}

/// 展开服务声明级 `#[lazy]`。字段同名 helper 由 injectable 消费，不进入此入口。
#[doc(hidden)]
pub fn expand_lazy(args: zyn::TokenStream, input: zyn::TokenStream) -> zyn::TokenStream {
    let result = ServiceLazyConfig::from_tokens(args)
        .and_then(|config| defer_to_provider(syn::parse2(input)?, config));
    result.unwrap_or_else(syn::Error::into_compile_error)
}

/// 展开迁移期保留的显式 `#[bind]`。
#[doc(hidden)]
pub fn expand_bind(args: zyn::TokenStream, input: zyn::TokenStream) -> zyn::TokenStream {
    expand_attribute(args, input, bind)
}

/// 保留原 `zyn::attribute` 的解析顺序和 FromInput 诊断，独立于 proc_macro ABI。
fn expand_attribute<T: zyn::FromInput>(
    args: zyn::TokenStream,
    input: zyn::TokenStream,
    expand: impl FnOnce(T, Args) -> zyn::Output,
) -> zyn::TokenStream {
    let item = match syn::parse2::<syn::Item>(input) {
        Ok(item) => item,
        Err(error) => return error.into_compile_error(),
    };
    let args = match syn::parse2::<Args>(args) {
        Ok(args) => args,
        Err(error) => return error.into_compile_error(),
    };
    match T::from_input(&zyn::Input::from(item)) {
        Ok(item) => expand(item, args).into(),
        Err(diagnostic) => diagnostic.emit(),
    }
}
