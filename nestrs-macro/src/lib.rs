//! Nestrs 的 proc macro 集。
//!
//! 与 `nestrs-core` 配套使用的宏（`injectable`、`factory`、`constructor`、
//! `primary`、`bind`）通过 `injection` feature 控制编译开关：
//!
//! ```toml
//! [dependencies]
//! nestrs-macro = { version = "...", default-features = false, features = ["injection"] }
//! ```

#[cfg(feature = "injection")]
mod injection;

mod utility;

use zyn::{
    meta::Args,
    syn::{self, spanned::Spanned},
    zyn,
};

#[cfg(feature = "injection")]
use crate::injection::{
    attrs::primary::{DeferPrimaryToInjectable, PrimaryConfig, take_primary_for_injectable},
    injectable::{
        CollectInjectableMetadata, DefineGenericInjectableComponent, EmitInjectableRegistration,
        GenerateInjectableConstructor, RewriteInjectionField, analyze_fields,
        config::InjectableConfig,
    },
};
#[cfg(feature = "injection")]
use crate::utility::{
    CheckConstructor, CheckInterfaceType, MustBePrivateFn, RejectUnsafeAndExternFn,
    RejectUnsafeImpl, RequireModuleScope, RequireNonUnitFutureOutputType,
    RequireNonUnitResultOkType, RequireNonUnitReturnType, ShouldBeAsyncFn, impl_self_ident,
};

#[cfg(feature = "injection")]
/// 将模块作用域内的结构体标记为可注入服务。
///
/// 字段来源由属性决定：`#[inject]` 从容器输入取得只读 `Inject<T>` 令牌；
/// `#[value(<Rust expression>)]` 则在词法隔离的隐藏构造 adapter 被 container
/// 调用时求值。因此它统一支持字面量、模块常量/静态项、可见路径、函数调用与普通
/// 组合表达式，并由 Rust 完成名称解析和类型检查。字符串字面量及模块常量/静态项
/// 会在需要时通过 `Into<字段类型>` 转换，因此 `String` 字段可直接写
/// `#[value("name")]`。
///
/// 该 adapter 不是类型成员且只由 linkme metadata 持有函数指针，因而用户不能以
/// `Service::__nestrs_construct(...)` 调用。由于它仍是非捕获函数，`#[value]` 不能
/// 引用调用点局部变量或另一字段；表达式必须能转换为字段类型。
#[zyn::attribute]
pub fn injectable(
    #[zyn(input)] item: syn::ItemStruct, // 被标注的项，自动提取
    args: Args,                          // #[injectable(...)] 里的原始参数
) -> zyn::TokenStream {
    // 1) 严格校验参数形态：只允许命名参数，且不允许重复
    let mut seen: Vec<String> = Vec::new();

    for arg in args.iter() {
        let Some(name) = arg.name() else {
            bail!(
                "`#[injectable]` 参数填写格式错误";
                span = arg.span()
            );
        };

        let name = name.to_string();
        if seen.contains(&name) {
            bail!("`#[injectable]` 参数 `{name}` 重复声明"; span = arg.span());
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
    let primary_attribute_use = primary.consumed_attribute_use();

    // 分析阶段只产出共享数据和去除 marker 的 AST；它不负责渲染后续阶段。
    let analyzed_fields = match analyze_fields(item) {
        Ok(fields) => fields,
        Err(error) => return error.into_compile_error().into(),
    };

    // 模块作用域检查所需的标识符必须在 zyn element 消费 AST 前保存。
    let scope_ident = Some(analyzed_fields.item.ident.clone());
    let is_open_generic_provider = !analyzed_fields.item.generics.params.is_empty();

    // cleanup 函数路径（可选）。保留 config 本身，供字段元数据收集器写入
    // provider 的 key 和 lifetime。
    let cleanup_path = config
        .cleanup
        .as_ref()
        .map(|cleanup| cleanup.func_path.clone());

    // 字段定义、构造 adapter 与 metadata 是三个独立的输出职责。注册 scope 只
    // 接收后两者作为 children，明确它们必须共享匿名词法作用域，避免把 helper
    // 暴露为用户可调用的 inherent method。
    zyn! {
        @RequireModuleScope(ident = scope_ident) {
            @if (cleanup_path.is_some()) {
                @ShouldBeAsyncFn(function_path = cleanup_path.clone().unwrap())
            }

            @RewriteInjectionField(
                analysis = analyzed_fields.clone(),
            )
            @if (is_open_generic_provider) {
                {{ primary_attribute_use }}
                @DefineGenericInjectableComponent(
                    analysis = analyzed_fields,
                    config = config,
                    primary = primary.is_primary(),
                )
            } @else {
                @EmitInjectableRegistration {
                    {{ primary_attribute_use }}
                    @GenerateInjectableConstructor(
                        analysis = analyzed_fields.clone(),
                    )
                    @CollectInjectableMetadata(
                        analysis = analyzed_fields,
                        config = config,
                        primary = primary.is_primary(),
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
#[cfg(feature = "injection")]
#[zyn::attribute]
pub fn factory(#[zyn(input)] item: syn::ItemFn, _args: Args) -> zyn::TokenStream {
    if item.sig.receiver().is_some() {
        return syn::Error::new(
            item.sig.ident.span(),
            "`#[factory]` 只能标注普通函数，不能用于带 `self` 的 impl 方法",
        )
        .into_compile_error()
        .into();
    }

    zyn! {
        @RejectUnsafeAndExternFn(macro_name = "factory".to_string(), item = item.clone()) {
            @RequireNonUnitReturnType(macro_name = "factory".to_string(), item = item.clone()) {
                @RequireNonUnitResultOkType(macro_name = "factory".to_string(), item = item.clone()) {
                    @RequireNonUnitFutureOutputType(macro_name = "factory".to_string(), item = item.clone()) {
                        @RequireModuleScope(ident = Some(item.sig.ident.clone())) {
                            @MustBePrivateFn() {
                                #[allow(dead_code)]
                                {{ item }}
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 将返回 `Self` 的无 `self` 函数声明为服务构造函数。
///
/// `#[constructor]` 不接受属性参数；被标记的函数必须不带 `self`、不能是
/// `unsafe` 或 `extern` 函数，并返回当前 impl 的类型。
///
/// 已弃用：当前不收集构造器元数据，请改用 `#[factory]` 自定义构造逻辑。
#[cfg(feature = "injection")]
#[deprecated(
    since = "0.1.0",
    note = "`#[constructor]` Rust暂不支持静态反射，还无法实现该功能，先留下口子，需要自定义构造请先使用 `#[factory]`"
)]
#[zyn::attribute]
pub fn constructor(#[zyn(input)] item: syn::ItemFn, args: Args) -> zyn::TokenStream {
    let macro_name = "constructor".to_owned();

    if let Some(arg) = args.iter().next() {
        return syn::Error::new(arg.span(), format!("`#[{macro_name}]` 不接受参数"))
            .into_compile_error()
            .into();
    }

    zyn! {
        @RejectUnsafeAndExternFn(macro_name = macro_name.clone(), item = item.clone()) {
            @CheckConstructor(macro_name = macro_name.clone()) {
                @MustBePrivateFn() {
                    {{ item }}
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
#[cfg(feature = "injection")]
#[zyn::attribute]
pub fn primary(#[zyn(input)] item: syn::Item, args: Args) -> zyn::TokenStream {
    let macro_name = "primary".to_owned();
    let primary_config = PrimaryConfig::from_args(&args);

    fn reject(span: ::zyn::proc_macro2::Span, message: &str) -> ::zyn::proc_macro2::TokenStream {
        syn::Error::new(span, message).into_compile_error()
    }

    zyn! {
        @match (primary_config) {
            Ok(primary) => {
                @match (item) {
                    // 函数标记：条件与 `#[factory]` 相同。
                    syn::Item::Fn(item) => {
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
                    // `primary` 位于 `injectable` 上方时，后者尚未展开。此
                    // element 追加 marker，交由 injectable 收集为 StructComponent::primary。
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

#[cfg(feature = "injection")]
#[zyn::attribute]
pub fn bind(
    #[zyn(input)] item: syn::ItemImpl, // 被标注的项，自动提取
    args: Args,                        // #[bind(...)] 里的原始参数
) -> zyn::TokenStream {
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

                    const _: () = {
                        fn __nestrs_project_bound_service(
                            service: &{{ service }}
                        ) -> &(dyn {{ interface }} + 'static) {
                            let projected: &(dyn {{ interface }} + 'static) = service;
                            projected
                        }

                        fn __nestrs_prepare_bound_required(
                            context: &mut ::nestrs_core::__private::ConstructionContext,
                            position: ::nestrs_core::__private::InputPosition,
                            input: ::core::option::Option<::nestrs_core::__private::ArenaServiceRef>,
                        ) -> ::core::result::Result<(), ::nestrs_core::__private::ActivationError> {
                            ::nestrs_core::__private::prepare_bound_required::<
                                {{ service }},
                                dyn {{ interface }},
                            >(
                                context,
                                position,
                                input,
                                __nestrs_project_bound_service,
                            )
                        }

                        fn __nestrs_prepare_bound_optional(
                            context: &mut ::nestrs_core::__private::ConstructionContext,
                            position: ::nestrs_core::__private::InputPosition,
                            input: ::core::option::Option<::nestrs_core::__private::ArenaServiceRef>,
                        ) -> ::core::result::Result<(), ::nestrs_core::__private::ActivationError> {
                            ::nestrs_core::__private::prepare_bound_optional::<
                                {{ service }},
                                dyn {{ interface }},
                            >(
                                context,
                                position,
                                input,
                                __nestrs_project_bound_service,
                            )
                        }

                        #[::nestrs_core::__private::linkme::distributed_slice(
                            ::nestrs_core::__private::REFLECT_METADATA_BIND
                        )]
                        #[linkme(crate = ::nestrs_core::__private::linkme)]
                        fn __nestrs_reflect_metadata_bind()
                            -> ::nestrs_core::__private::InterfaceBinding
                        {
                            ::nestrs_core::__private::InterfaceBinding {
                                service_type: ::nestrs_core::registration::service_type::ServiceType::create::<{{ service }}>(),
                                trait_type: ::nestrs_core::registration::service_type::ServiceType::create::<dyn {{ interface }}>(),
                                prepare_required: __nestrs_prepare_bound_required,
                                prepare_optional: __nestrs_prepare_bound_optional,
                            }
                        }

                        ()
                    };
                }
            }
        }
    }
}
