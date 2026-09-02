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
use crate::injection::injectable::config::InjectableConfig;
#[cfg(feature = "injection")]
use crate::utility::{
    impl_self_ident, CheckConstructor, CheckInterfaceType, MustBePrivateFn, RequireModuleScope,
    RejectUnsafeAndExternFn, RejectUnsafeImpl, ShouldBeAsyncFn,
};


#[cfg(feature = "injection")]
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

    println!("[调试]：解析 `#[injectable]` 参数成功，配置为：\n{config:#?}");

    // 结构体名称
    let struct_name = item.ident.clone();

    let cleanup_path = config.cleanup.map(|cleanup| cleanup.func_path);

    zyn! {
        @RequireModuleScope(ident = Some(item.ident.clone())) {
            @if (cleanup_path.is_some()) {
                @ShouldBeAsyncFn(function_path = cleanup_path.clone().unwrap())
            }

            {{ item }}
        }
    }
}

/// 将模块作用域内的私有函数声明为服务工厂函数。
///
/// 支持同步与 `async` 函数；不允许带 `self`、`unsafe` 或 `extern` 的函数。
#[cfg(feature = "injection")]
#[zyn::attribute]
pub fn factory(
    #[zyn(input)] item: syn::ItemFn,
    _args: Args,
) -> zyn::TokenStream {
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
            @RequireModuleScope(ident = Some(item.sig.ident.clone())) {
                @MustBePrivateFn() {
                    {{ item }}
                }
            }
        }
    }
}

/// 将返回 `Self` 的无 `self` 函数声明为服务构造函数。
///
/// `#[constructor]` 不接受属性参数；被标记的函数必须不带 `self`、不能是
/// `unsafe` 或 `extern` 函数，并返回当前 impl 的类型。
/// 运行时注册元数据将在后续阶段生成。
#[cfg(feature = "injection")]
#[zyn::attribute]
pub fn constructor(
    #[zyn(input)] item: syn::ItemFn,
    args: Args,
) -> zyn::TokenStream {
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
pub fn primary(
    #[zyn(input)] item: syn::Item,
    args: Args,
) -> zyn::TokenStream {
    let macro_name = "primary".to_owned();

    fn reject(span: ::zyn::proc_macro2::Span, message: &str) -> ::zyn::proc_macro2::TokenStream {
        syn::Error::new(span, message).into_compile_error()
    }

    zyn! {
        @match (args.iter().next()) {
            Some(arg) => {
                {{ reject(arg.span(), "`#[primary]` 不接受参数") }}
            }
            None => {
                @match (item) {
                    // 函数标记：条件与 `#[factory]` 相同
                    syn::Item::Fn(item) => {
                        @if (item.sig.receiver().is_some()) {
                            {{ reject(item.sig.ident.span(), "`#[primary]` 只能标注普通函数，不能用于带 `self` 的 impl 方法") }}
                        } @else {
                            @RejectUnsafeAndExternFn(macro_name = macro_name, item = item.clone()) {
                                @RequireModuleScope(ident = Some(item.sig.ident.clone())) {
                                    @MustBePrivateFn() {
                                        {{ item }}
                                    }
                                }
                            }
                        }
                    }
                    // 结构体标记：条件与 `#[injectable]` 相同
                    syn::Item::Struct(item) => {
                        @RequireModuleScope(ident = Some(item.ident.clone())) {
                            {{ item }}
                        }
                    }
                    other => {
                        {{ reject(other.span(), "`#[primary]` 只能标注普通函数或结构体") }}
                    }
                }
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
                            }
                        }
                    };
                }
            }
        }
    }
}
