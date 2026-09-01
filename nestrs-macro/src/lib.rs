mod injection;
mod utility;

use zyn::{
    meta::Args,
    syn::{self, spanned::Spanned},
    zyn,
};

use crate::injection::{injectable::config::InjectableConfig, primary::config::PrimaryConfig};
use crate::utility::{
    CheckConstructor, CheckInterfaceType, MustBePrivateFn, RequireModuleScope, ShouldBeAsyncFn,
};


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

        @if (cleanup_path.is_some()) {
            @ShouldBeAsyncFn(function_path = cleanup_path.clone().unwrap())
        }

        {{ item }}
    }
}

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
        @RequireModuleScope() {
            @MustBePrivateFn() {
                {{ item }}
            }
        }
    }
}

/// 将返回 `Self` 的无 `self` 函数声明为服务构造函数。
///
/// `#[constructor]` 不接受属性参数；被标记的函数必须不带 `self`，并返回当前 impl 的类型。
/// 运行时注册元数据将在后续阶段生成。
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
        @CheckConstructor(macro_name = macro_name) {
            @MustBePrivateFn() {
                {{ item }}
            }
        }
    }
}

#[zyn::attribute]
pub fn primary(
    #[zyn(input)] item: syn::ItemStruct, // 被标注的项，自动提取
    args: Args,                          // #[primary(...)] 里的原始参数
) -> zyn::TokenStream {
    // 2) 强类型解析参数；失败时直接发射诊断
    let config = match PrimaryConfig::from_args(&args) {
        Ok(cfg) => cfg,
        Err(diag) => return diag.emit().into(),
    };

    println!("[调试]：解析 `#[primary]` 参数成功，配置为：\n{config:#?}");

    let trait_names = &config.trait_names.trait_names;

    zyn! {
        {{ item }}
        @for (interface in trait_names) {
            @CheckInterfaceType(interface = interface.clone())
        }
    }
}

#[zyn::attribute]
pub fn bind(
    #[zyn(input)] item: syn::ItemImpl, // 被标注的项，自动提取
    args: Args,                        // #[bind(...)] 里的原始参数
) -> zyn::TokenStream {
    zyn! {
        {{ item }}
    }
}
